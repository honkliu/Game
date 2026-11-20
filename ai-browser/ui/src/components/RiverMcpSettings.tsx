import { useEffect, useState } from 'preact/hooks';
import { native } from '../store';

interface RiverMcpState {
  port: number;
  token: string;
  active_port: number;
  listen_address: string;
  error?: string;
}

export function RiverMcpSettings() {
  const [state, setState] = useState<RiverMcpState>();
  const [port, setPort] = useState('');
  const [token, setToken] = useState('');
  const [visible, setVisible] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState({ text: '', error: false });
  useEffect(() => {
    let active = true;
    void native.invoke<RiverMcpState>('shell_settings', { operation: 'river_mcp_get' }).then(next => {
      if (!active) return;
      setState(next); setPort(String(next.port)); setToken(next.token);
      if (next.error) setMessage({ text: next.error, error: true });
    }).catch(error => { if (active) setMessage({ text: String(error), error: true }); });
    return () => { active = false; };
  }, []);
  const edit = (fn: () => void) => { fn(); setMessage({ text: '', error: false }); };
  const apply = async () => {
    setBusy(true); setMessage({ text: 'Applying...', error: false });
    try {
      const next = await native.invoke<RiverMcpState>('shell_settings', {
        operation: 'river_mcp_apply', value: { port: Number(port), token },
      });
      setState(next);
      setMessage({ text: `Applied. Listening on ${next.listen_address}:${next.active_port}.`, error: false });
    } catch (error) {
      setMessage({ text: error instanceof Error ? error.message : String(error), error: true });
    } finally { setBusy(false); }
  };
  return <section class="ag-card" aria-label="RiverMCP server">
    <p class="ag-help">Connect external agents to River. The browser UI and AI process use the same port and token automatically.</p>
    <p class="ag-help">On startup, River tries port 3000, then uses a random available port if occupied. Apply changes the port for the current run.</p>
    <label class="ag-field">Listen address<input value="0.0.0.0" readOnly /></label>
    <label class="ag-field">Port<input id="rivermcp-port" type="number" min={1} max={65535} value={port}
      disabled={!state || busy} onInput={event => edit(() => setPort(event.currentTarget.value))} /></label>
    <label class="ag-field">Token<input id="rivermcp-token" {...(visible ? { type: 'text' as const } : { type: 'password' as const })} value={token} autoComplete="off" spellcheck={false}
      disabled={!state || busy} onInput={event => edit(() => setToken(event.currentTarget.value))} /></label>
    <div><button type="button" class="chip-btn" disabled={!state || busy} onClick={() => setVisible(!visible)}>{visible ? 'Hide token' : 'Show token'}</button>{' '}
      <button type="button" class="chip-btn" disabled={!state || busy} onClick={() => edit(() => {
        const bytes = crypto.getRandomValues(new Uint8Array(32));
        setToken(Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join(''));
      })}>Generate token</button></div>
    {state ? <p class="ag-help">Listening on {state.listen_address}:{state.active_port}. Local endpoint: <code>http://127.0.0.1:{state.active_port}/mcp</code>. For another computer, use this computer's IP address.</p> : null}
    <div><button id="rivermcp-apply" type="button" class="chip-btn is-accent" disabled={!state || busy} onClick={() => void apply()}>Apply</button>{' '}
      <span id="rivermcp-result" role={message.error ? 'alert' : 'status'} aria-live="polite" class={'ask-status' + (message.error ? ' is-error' : '')}>{message.text}</span></div>
  </section>;
}
