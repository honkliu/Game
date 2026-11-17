import type { ModelInfo } from './types';

export type ModelCommand = { kind: 'select'; id: string; label: string }
  | { kind: 'list'; query: string; error?: string };

export function modelUnavailable(model: ModelInfo, agent: boolean): string {
  if (model.available === false) return model.unavailable_reason || 'unavailable';
  if (agent && (model.supports_tools === false || model.provider_id === 'extractive')) return 'questions only';
  return '';
}

export function modelCommand(text: string, models: ModelInfo[], agent: boolean): ModelCommand | null {
  const command = /^\s*\/model(?:\s+([\s\S]*))?$/i.exec(text);
  if (!command) return null;
  const query = (command[1] || '').trim();
  if (!query) return { kind: 'list', query: '' };
  if (query.toLowerCase() === 'default') return { kind: 'select', id: '', label: 'Default' };
  const full = models.filter(model => `${model.provider_id}/${model.model_id}` === query);
  const exact = models.filter(model => model.model_id === query);
  const matches = full.length ? full : exact.length ? exact
    : models.filter(model => model.display_name?.toLowerCase() === query.toLowerCase());
  if (matches.length !== 1) return {
    kind: 'list', query, error: matches.length ? 'Several models match. Choose one below.' : 'No matching model. Choose one or add it in Settings.',
  };
  const model = matches[0]!;
  const reason = modelUnavailable(model, agent);
  if (reason) return { kind: 'list', query, error: `This model is ${reason}.` };
  return { kind: 'select', id: `${model.provider_id}/${model.model_id}`, label: model.display_name || model.model_id };
}
