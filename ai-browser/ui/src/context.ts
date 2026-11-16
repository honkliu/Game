// What the assistant is given about the browser, and when a user's edit of it stops
// being valid. All pure, so the rules are unit-tested without a browser.

import type { AgentContext, ContextEcho } from './types';

/** A fingerprint of everything the generated context is built from: the active page and
 *  its URL, the content revision the shell read, the selection, and the tab list. When it
 *  changes, a context the user edited no longer describes the page. */
export function contextSignature(c: AgentContext | null | undefined): string {
  const p = c?.page;
  if (!p) return 'none';
  const tabs = (c?.tabs ?? []).map((t) => `${t.tab_id}@${t.url}`).join(',');
  return [p.tab_id, p.url, c?.content?.rev ?? 0, p.selection ?? '', tabs].join('\u0001');
}

export interface ContextEdit {
  text: string;
  /** The signature of the context the edit was made against. */
  sig: string;
}

/** The edit survives only while the context it was based on is still current. A newer
 *  page state discards it (skills, prompts and settings are unaffected: they are stored). */
export function reconcileEdit(edit: ContextEdit | null, currentSig: string): { edit: ContextEdit | null; discarded: boolean } {
  if (!edit) return { edit: null, discarded: false };
  if (edit.sig === currentSig) return { edit, discarded: false };
  return { edit: null, discarded: true };
}

export function echoOf(c: AgentContext | null | undefined, edited: boolean): ContextEcho {
  const p = c?.page;
  return {
    title: p?.title || p?.url || 'No page open',
    url: p?.url ?? '',
    selectionChars: p?.selection?.length ?? 0,
    tabs: c?.tabs?.length ?? 0,
    contentChars: c?.content?.chars ?? null,
    edited,
  };
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return '';
  }
}

/** "3 s ago"-style age for the live indicator. */
export function ageLabel(at: number, now: number): string {
  const s = Math.max(0, Math.round((now - at) / 1000));
  if (s < 5) return 'just now';
  if (s < 60) return `${s} s ago`;
  return `${Math.round(s / 60)} min ago`;
}
