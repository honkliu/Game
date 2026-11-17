import { useEffect, useRef, useState } from 'preact/hooks';
import { native } from '../store';

interface Entry { id: number; url: string; title: string; host: string; visited_at: number; visit_count: number }

export function BrowserHistory() {
  const [query, setQuery] = useState('');
  const [rows, setRows] = useState<Entry[]>([]);
  const [error, setError] = useState('');
  const [range, setRange] = useState('all');
  const [selected, setSelected] = useState<number[]>([]);
  const sequence = useRef(0);
  const load = async () => {
    const current = ++sequence.current;
    try {
      const result = await native.invoke<{ rows: Entry[] }>('history.query', { tokens: query });
      if (current === sequence.current) { setRows(result.rows); setError(''); setSelected([]); }
    } catch (e) { if (current === sequence.current) setError(String(e)); }
  };
  useEffect(() => {
    const timeout = setTimeout(() => void load(), 180);
    const stop = native.on('state.history_meta', () => void load());
    return () => { clearTimeout(timeout); stop(); sequence.current++; };
  }, [query]);
  const remove = async (payload: object) => {
    try { await native.invoke('history.forget', payload); }
    catch (e) { setError(String(e)); }
  };
  const clear = async () => {
    if (!confirm('Clear this browsing history?')) return;
    try {
      const to = Math.floor(Date.now() / 1000);
      await native.invoke('history.clear', range === 'all' ? { scope: 'all' } : { scope: 'range', from: to - Number(range), to });
    } catch (e) { setError(String(e)); }
  };
  return <>
    <input class="ag-search" type="search" aria-label="Search history" placeholder="Search history (text, site:example.com, after:YYYY-MM-DD)" value={query} onInput={event => setQuery(event.currentTarget.value)} />
    <div class="settings-actions">
      <button type="button" class="chip-btn" disabled={!selected.length} onClick={() => void remove({ entry_ids: selected })}>Delete selected</button>
      <select aria-label="History range" value={range} onChange={event => setRange(event.currentTarget.value)}>
        <option value="all">All time</option><option value="3600">Last hour</option><option value="86400">Last day</option><option value="604800">Last week</option>
      </select>
      <button type="button" class="chip-btn is-danger" onClick={() => void clear()}>Clear history</button>
    </div>
    {error ? <p class="ag-error" role="alert">{error}</p> : null}
    {!rows.length ? <p class="overlay-empty">No matching history.</p> : null}
    {rows.map((entry, index) => {
      const date = new Date(entry.visited_at * 1000).toLocaleDateString();
      const previous = index ? new Date(rows[index - 1]!.visited_at * 1000).toLocaleDateString() : '';
      return <section key={entry.id}>
        {date !== previous ? <h2>{date}</h2> : null}
        <div class="ag-row browser-history-entry">
          <input type="checkbox" aria-label={`Select ${entry.title}`} checked={selected.includes(entry.id)}
            onChange={event => setSelected(current => event.currentTarget.checked ? [...current, entry.id] : current.filter(id => id !== entry.id))} />
          <button type="button" class="ag-row-main" title={entry.url} onClick={() => void native.invoke('nav.open', { url: entry.url, disposition: 'new_tab' }).catch(e => setError(String(e)))}>
            <b>{entry.title || entry.host}</b>
          </button>
          <button type="button" class="chip-btn" onClick={() => void remove({ site: entry.host })}>Forget site</button>
          <button type="button" class="chip-btn is-danger" onClick={() => void remove({ entry_ids: [entry.id] })}>Delete</button>
        </div>
      </section>;
    })}
  </>;
}
