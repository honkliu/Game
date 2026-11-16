import { agentOn, agentState, navAuto, newChat, openSettings, saveAsSkill, sendMessage, setAgentOn, setNavAuto, view } from '../store';
import { native } from '../store';
import { Transcript } from './Transcript';
import { ContextBar } from './ContextBar';
import { Composer } from './Composer';
import { Settings } from './Settings';
import { History } from './History';

/** A grip on the pane's left edge. The pane is its own webview and moves as it resizes,
 *  so the drag is measured in screen coordinates and the shell clamps and applies it. */
function ResizeGrip() {
  const down = (e: PointerEvent) => {
    const grip = e.currentTarget as HTMLElement;
    grip.setPointerCapture(e.pointerId);
    const startX = e.screenX;
    const startW = window.innerWidth;
    let raf = 0;
    let wanted = startW;
    const move = (m: PointerEvent) => {
      wanted = startW + (startX - m.screenX);
      if (!raf) raf = requestAnimationFrame(() => { raf = 0; void native.invoke('ai.pane.resize', { width: wanted }); });
    };
    const up = () => {
      grip.removeEventListener('pointermove', move);
      grip.removeEventListener('pointerup', up);
      grip.removeEventListener('pointercancel', up);
    };
    grip.addEventListener('pointermove', move);
    grip.addEventListener('pointerup', up);
    grip.addEventListener('pointercancel', up);
  };
  return <div id="resize-grip" role="separator" aria-orientation="vertical" aria-label="Resize AI pane" title="Drag to resize" onPointerDown={down} />;
}

function Header() {
  const on = agentOn.value;
  return (
    <div id="mode-bar">
      <strong class="bar-title">✦ AI</strong>
      <label class="switch" title="Agent mode: the assistant can use tools, browse and act. Off: ask about the page and get cited answers.">
        <input type="checkbox" id="agent-switch" role="switch" aria-label="Agent mode" checked={on} onChange={(e) => setAgentOn(e.currentTarget.checked)} />
        <span class="switch-track" />
        <span class="switch-label">Agent</span>
      </label>
      {on ? (
        <label class="switch" title="Auto: the assistant opens pages in the main view by itself. Manual: it shows a link and you click it.">
          <input type="checkbox" id="nav-switch" role="switch" aria-label="Auto navigate" checked={navAuto.value} onChange={(e) => setNavAuto(e.currentTarget.checked)} />
          <span class="switch-track" />
          <span class="switch-label">{navAuto.value ? 'Auto' : 'Manual'}</span>
        </label>
      ) : null}
      <span class="bar-fill" />
      <button type="button" class="chip-btn" onClick={newChat}>New</button>
      {on ? <button type="button" class="chip-btn" onClick={() => (view.value = 'history')}>History</button> : null}
      <button type="button" class="chip-btn" onClick={() => openSettings('skills')}>Skills</button>
      <button type="button" class="chip-btn" onClick={() => openSettings('prompts')}>⚙</button>
    </div>
  );
}

function Chat() {
  const state = agentState.value;
  const mcp = state ? state.mcp.filter((m) => m.status === 'connected') : [];
  const tools = mcp.reduce((n, m) => n + m.tools.length, 0);
  return (
    <div class="chat">
      <Transcript onStarter={(text) => void sendMessage(text)} />
      <ContextBar />
      <Composer />
      {agentOn.value ? (
        <div class="ag-foot">
          <button type="button" id="ag-mcp" class="ag-chip" onClick={() => openSettings('mcp')}>
            {state ? (state.mcp.length ? `MCP: ${mcp.length}/${state.mcp.length} servers · ${tools} tools` : 'MCP: add a server') : 'MCP'}
          </button>
          <button type="button" class="ag-chip" onClick={() => void saveAsSkill()}>Save chat as skill</button>
          <button type="button" class="ag-chip" onClick={() => openSettings('prompts')}>Edit prompts</button>
        </div>
      ) : null}
    </div>
  );
}

export function App() {
  const v = view.value;
  return (
    <>
      <ResizeGrip />
      <Header />
      {v === 'settings' ? <Settings /> : v === 'history' ? <History /> : <Chat />}
    </>
  );
}
