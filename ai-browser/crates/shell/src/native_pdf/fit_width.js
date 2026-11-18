(() => {
  const viewport = globalThis.viewer?.viewport_;
  if (typeof viewport?.fitToWidth !== 'function') {
    throw new Error('Native PDF fit-to-width operation unavailable.');
  }
  viewport.fitToWidth();
  return 'fit-width';
})()
