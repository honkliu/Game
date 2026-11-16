import { render } from 'preact';
import { App } from './components/App';
import { startStore } from './store';
import './styles.css';

startStore();
render(<App />, document.getElementById('app')!);
