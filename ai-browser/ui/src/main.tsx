import { render } from 'preact';
import { App } from './components/App';
import { startStore } from './store';
import { startSettingsStore } from './settings_store';
import { SettingsPage } from './components/SettingsPage';
import './styles.css';

const settings = document.body.dataset.mode === 'settings';
if (settings) startSettingsStore();
else startStore();
render(settings ? <SettingsPage /> : <App />, document.getElementById('app')!);
