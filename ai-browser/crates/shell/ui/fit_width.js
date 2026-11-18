(request, documentToken, zoom, previousWidth) => {
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const root = document.documentElement;
    const width = Math.max(root.scrollWidth, document.body?.scrollWidth || 0);
    if (root.clientWidth <= 1 || width <= root.clientWidth) return;
    window.ipc.postMessage(JSON.stringify({
      page: 'fit_width',
      request,
      document_token: documentToken,
      width: root.clientWidth,
      limited: root.clientWidth <= previousWidth,
      factor: zoom * (root.clientWidth - 1) / width,
    }));
  }));
}
