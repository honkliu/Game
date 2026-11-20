import { useEffect, useRef } from 'preact/hooks';
import {
  agentOn, aiStatus, busy, candidates, extraTabs, focusTick, intent, localOnly, model, models, native, sendMessage,
  modelFilter, modelMenu, promptHistory, status, stop, turns, useSelection, type Intent,
} from '../store';
import { ModelPicker } from './ModelPicker';

const QUICK: [string, { intent: Intent; selection?: boolean; question?: string }][] = [
  ['Summarize page', { intent: 'summarize' }],
  ['Key points', { intent: 'ask', question: 'List the key points of this page.' }],
  ['Explain selection', { intent: 'explain', selection: true }],
  ['Find related pages', { intent: 'act', question: 'Find pages related to this one and open the best two as candidate tabs.' }],
];

export function Composer() {
  const ref = useRef<HTMLTextAreaElement>(null);
  const historyIndex = useRef<number | null>(null);
  const draft = useRef('');
  const on = agentOn.value;
  const isBusy = busy.value;
  const s = aiStatus.value;
  const pre = on ? 'ag' : 'ask';

  useEffect(() => {
    if (!isBusy) ref.current?.focus();
  }, [focusTick.value, isBusy, on]);

  const submit = () => {
    const el = ref.current;
    if (!el || !el.value.trim()) return;
    const text = el.value;
    el.value = '';
    historyIndex.current = null;
    void sendMessage(text);
  };
  // Ask mode may send with an empty box when a selection and a verb are chosen.
  const submitAsk = () => {
    const el = ref.current;
    const text = el?.value ?? '';
    if (el) el.value = '';
    historyIndex.current = null;
    void sendMessage(text);
  };

  const others = (s?.multi_tab && s.other_tabs) || [];
  return (
    <div class="composer">
      {!on ? (
        <div id="ask-quick" class="ask-quick">
          {QUICK.map(([label, over]) => (
            <button type="button" class="ask-qbtn" key={label} disabled={isBusy} onClick={() => void sendMessage(over.question ?? '', over)}>{label}</button>
          ))}
        </div>
      ) : null}
      <div id={pre + '-status'} class={'ask-status' + (status.value.kind ? ' is-' + status.value.kind : '')} role="status">{status.value.text}</div>
      <div class="ask-head">
        <textarea id={pre + '-input'} ref={ref} class="ask-input" rows={2} spellcheck={false} disabled={isBusy}
          placeholder={on ? 'Ask the agent to do something. It can read tabs, browse, search and use tools.' : 'Ask a question about this page'}
          onKeyDown={(e) => {
            if (e.isComposing) return;
            const el = e.currentTarget;
            if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey && el.selectionStart === el.selectionEnd) {
              const up = e.key === 'ArrowUp';
              const atEdge = up ? !el.value.slice(0, el.selectionStart).includes('\n') : !el.value.slice(el.selectionEnd).includes('\n');
              const history = promptHistory.value.length ? promptHistory.value : turns.value.map(turn => turn.question).filter(text => !!text.trim());
              if (atEdge && history.length && (up || historyIndex.current !== null)) {
                e.preventDefault();
                if (historyIndex.current === null) draft.current = el.value;
                const next = Math.max(0, Math.min(history.length, (historyIndex.current ?? history.length) + (up ? -1 : 1)));
                historyIndex.current = next === history.length ? null : next;
                el.value = history[next] ?? draft.current;
                const caret = up ? 0 : el.value.length;
                el.setSelectionRange(caret, caret);
                return;
              }
            }
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              if (on) submit();
              else submitAsk();
            }
          }} />
        <button id={pre + '-send'} type="button" class="chip-btn is-accent" hidden={isBusy} onClick={on ? submit : submitAsk}>{on ? 'Send' : 'Ask'}</button>
        <button id={pre + '-stop'} type="button" class="chip-btn" hidden={!isBusy} onClick={() => void stop()}>Stop</button>
      </div>
      <div class="ask-opts"><button type="button" class="chip-btn" aria-label="Change model" disabled={isBusy}
        onClick={() => { modelFilter.value = ''; modelMenu.value = !modelMenu.value; }}>
        Model: {models.value.find(entry => `${entry.provider_id}/${entry.model_id}` === model.value)?.display_name || model.value || 'Default'} <small>/model</small>
      </button></div>
      {modelMenu.value ? <ModelPicker /> : null}
      {!on ? (
        <>
          <div class="ask-opts">
            <select id="ask-intent" aria-label="Task" value={intent.value} onChange={(e) => (intent.value = e.currentTarget.value as Intent)}>
              {(['ask', 'summarize', 'explain', 'verify', 'translate', 'rewrite'] as Intent[]).map((k) => <option key={k} value={k}>{k.charAt(0).toUpperCase() + k.slice(1)}</option>)}
              <option value="act">Act (may open tabs)</option>
            </select>
            <label class="ask-opt">
              <input id="ask-sel" type="checkbox" checked={useSelection.value} disabled={!s?.selection} onChange={(e) => (useSelection.value = e.currentTarget.checked)} /> selection
            </label>
            <label class="ask-opt">
              <input id="ask-local" type="checkbox" checked={localOnly.value} onChange={(e) => (localOnly.value = e.currentTarget.checked)} /> local only
            </label>
          </div>
          {others.length ? (
            <div id="ask-tabs" class="ask-tabs" aria-label="Also read these tabs">
              <span class="ask-tabs-lead">Also read:</span>
              {others.map((t) => (
                <label class="ask-opt ask-tab-opt" key={t.tab_id}>
                  <input type="checkbox" data-tab={String(t.tab_id)} checked={extraTabs.value.includes(t.tab_id)}
                    onChange={(e) => (extraTabs.value = e.currentTarget.checked ? [...extraTabs.value, t.tab_id] : extraTabs.value.filter((x) => x !== t.tab_id))} />
                  {' ' + (t.title || t.host || 'tab').slice(0, 28)}
                </label>
              ))}
            </div>
          ) : null}
          <div id="ask-cands" class="ask-cands">
            {candidates.value.map((c) => (
              <button type="button" class="ask-chip ask-cand" key={c.tab_id} title={(c.reason ? c.reason + ' — ' : '') + (c.url || '')}
                onClick={() => native.invoke('shell_tab', { operation: 'activate', tab_id: c.tab_id }).catch(() => undefined)}>
                {(c.title || c.url || 'candidate').slice(0, 40)}
              </button>
            ))}
          </div>
        </>
      ) : null}
    </div>
  );
}
