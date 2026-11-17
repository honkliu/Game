import { useEffect, useState } from 'preact/hooks';
import { deleteConversation, listConversations, native } from '../store';
import type { ConversationRow } from '../types';

export function History() {
  const [q, setQ] = useState('');
  const [rows, setRows] = useState<ConversationRow[] | null>(null);
  const [error, setError] = useState('');
  const load = async (query: string) => {
    try {
      setRows(await listConversations(query));
      setError('');
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  useEffect(() => {
    const id = setTimeout(() => void load(q), 200);
    return () => clearTimeout(id);
  }, [q]);
  return (
    <div class="saved-chats">
      <div class="ag-pad"><input type="text" class="ag-search" placeholder="Search conversations" value={q} onInput={(e) => setQ(e.currentTarget.value)} /></div>
      <div class="ag-list">
        {error ? <p class="ag-error">{error}</p> : rows === null ? <p class="overlay-empty">Loading…</p> : !rows.length ? <p class="overlay-empty">No saved conversations yet.</p> : rows.map((c) => (
          <div class="ag-row" key={c.id}>
            <button type="button" class="ag-row-main" onClick={() => void native.invoke('ai.conversation.open', { id: c.id }).catch(e => setError(String(e)))}>
              <b>{c.title || c.id}</b><small>{c.updated ? new Date(c.updated).toLocaleString() : ''}</small>
            </button>
            <button type="button" class="chip-btn is-danger" onClick={() => void deleteConversation(c.id).catch(() => undefined).then(() => load(q))}>Delete</button>
          </div>
        ))}
      </div>
    </div>
  );
}
