// Runs only in the owning WebView's built-in PDF extension, never in page content.
(() => new Promise((resolve, reject) => {
  const controller = globalThis.viewer?.pluginController_;
  const port = controller?.messagePort_;
  if (typeof controller?.postMessage !== 'function' ||
      typeof port?.addEventListener !== 'function' ||
      typeof port?.removeEventListener !== 'function') {
    reject(new Error('Native PDF selection protocol unavailable.'));
    return;
  }
  const cleanup = () => {
    clearTimeout(timer);
    port.removeEventListener('message', receive);
  };
  const receive = event => {
    if (event.data?.type !== 'getSelectedTextReply') return;
    cleanup();
    if (typeof event.data.selectedText !== 'string') {
      reject(new Error('Invalid native PDF selection reply.'));
      return;
    }
    resolve(event.data.selectedText);
  };
  const timer = setTimeout(() => {
    cleanup();
    reject(new Error('Native PDF selection timed out.'));
  }, 1000);
  port.addEventListener('message', receive);
  try {
    controller.postMessage({ type: 'getSelectedText' });
  } catch (error) {
    cleanup();
    reject(error);
  }
}))()
