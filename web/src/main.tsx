import { render } from 'solid-js/web';
import { App } from './app/App';
import { warmEngine } from './engine/client';
import { loadOtherStreams } from './engine/streams';
import './styles.css';
import { installTooltips } from './lib/tooltip';

// On the old address public/moved.js, read before this, is already sending the visitor on: nothing to start.
const leaving = ['tradoor.rsalianto.com', 'www.tradoorterminal.com'].includes(location.hostname);
if (!leaving) start();

function start(): void {
  installTooltips();
  render(() => <App />, document.getElementById('root')!);

  // What every first screen needs, begun before the layout has loaded to ask for
  // it — once the page is up, so a failure here costs the head start and not the
  // page: a broken lazy import once threw here and left every new visitor blank.
  for (const warm of [warmEngine, loadOtherStreams]) {
    try {
      warm();
    } catch (e) {
      console.warn('tradoor: could not start early', e);
    }
  }
}
