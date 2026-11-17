import { useEffect, useRef } from 'preact/hooks';
import { agentOn, aiStatus, focusTick, model, modelFilter, modelMenu, models, openSettings, setModel, setStatus } from '../store';
import { modelUnavailable } from '../model_commands';

export function ModelPicker() {
  const ref = useRef<HTMLInputElement>(null);
  const query = modelFilter.value.toLowerCase();
  const choices = models.value.filter(entry => `${entry.provider_id}/${entry.model_id} ${entry.display_name || ''}`.toLowerCase().includes(query));
  useEffect(() => { ref.current?.focus(); }, []);
  const choose = (id: string, label: string) => {
    setModel(id);
    modelMenu.value = false;
    setStatus(`Model: ${label}`);
    focusTick.value++;
  };
  const key = (event: KeyboardEvent) => {
    const buttons = [...(event.currentTarget as HTMLDivElement).querySelectorAll<HTMLButtonElement>('[role="option"]:not(:disabled)')];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === 'Escape') { event.preventDefault(); modelMenu.value = false; focusTick.value++; }
    else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const down = event.key === 'ArrowDown';
      buttons[index < 0 ? (down ? 0 : buttons.length - 1) : (index + (down ? 1 : -1) + buttons.length) % buttons.length]?.focus();
    } else if (event.key === 'Enter' && document.activeElement === ref.current) {
      event.preventDefault();
      buttons[0]?.click();
    }
  };
  return (
    <div class="model-picker" onKeyDown={key}>
      <label class="ag-field">Choose a model<input ref={ref} type="search" aria-label="Search models" value={modelFilter.value}
        onInput={event => { modelFilter.value = event.currentTarget.value; }} /></label>
      <div role="listbox" aria-label="Models">
        {!query ? <button type="button" role="option" aria-selected={!model.value} data-model=""
          onClick={() => choose('', 'Default')}>Default</button> : null}
        {choices.map(entry => {
          const id = `${entry.provider_id}/${entry.model_id}`;
          const reason = modelUnavailable(entry, agentOn.value);
          const endpoint = aiStatus.value?.provider?.connections?.find(connection => connection.id === entry.provider_id)?.base_url;
          return <button type="button" role="option" key={id} data-model={id} aria-selected={model.value === id}
            disabled={!!reason} onClick={() => choose(id, entry.display_name || entry.model_id)}>
            <b>{entry.display_name || entry.model_id}</b>
            <small>{reason || endpoint || entry.provider_id}</small>
          </button>;
        })}
        {!choices.length ? <p class="ag-help">No matching models.</p> : null}
      </div>
      <button type="button" class="chip-btn" onClick={() => { modelMenu.value = false; openSettings('model'); }}>Manage models</button>
    </div>
  );
}
