import { agentOn, agentState, navAuto, newChat, openSettings, saveAsSkill, sendMessage, setAgentOn, setNavAuto } from '../store';
import { native } from '../store';
import { Transcript } from './Transcript';
import { ContextBar } from './ContextBar';
import { Composer } from './Composer';

const assistantName = /^zh(?:-|$)/i.test(navigator.language) ? '洛' : 'River';

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
      if (!raf) raf = requestAnimationFrame(() => { raf = 0; void native.invoke('shell_ai', { operation: 'pane_resize', width: wanted }); });
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
      <strong class="bar-title">✦ {assistantName}</strong>
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
      <button type="button" class="chip-btn" aria-label="Settings" title="Settings" onClick={() => openSettings('general')}>⚙</button>
    </div>
  );
}

function Chat() {
  const state = agentState.value;
  const tools = state?.tools || [];
  return (
    <div class="chat">
      <Transcript onStarter={(text) => void sendMessage(text)} />
      <ContextBar />
      <Composer />
      {agentOn.value ? (
        <div class="ag-foot">
          <button type="button" id="ag-tools" class="ag-chip" onClick={() => openSettings('tools')}>
            {state ? state.tools_enabled === false ? 'Tools: off' : `Tools: on (${tools.length})` : 'Tools'}
          </button>
          <button type="button" class="ag-chip" onClick={() => void saveAsSkill()}>Save chat as skill</button>
          <button type="button" class="ag-chip" onClick={() => openSettings('prompts')}>Edit prompts</button>
        </div>
      ) : null}
    </div>
  );
}

export function App() {
  return (
    <>
      <ResizeGrip />
      <Header />
      <Chat />
    </>
  );
}
