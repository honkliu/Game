(function () {
  function install() {
    var probe = document.createElement('div');
    probe.style.cssText = 'all:initial;position:fixed;left:-10000px;top:-10000px;width:100px;height:100px;border:0;padding:0;overflow:scroll;visibility:hidden;contain:strict';
    document.documentElement.appendChild(probe);
    var thickness = Math.max(probe.offsetWidth - probe.clientWidth, probe.offsetHeight - probe.clientHeight);
    probe.remove();
    // Keep overlay and already-thin scrollbars from gaining a larger gutter.
    if (thickness <= 8) return;
    var sheet = new CSSStyleSheet();
    sheet.replaceSync('@layer river-browser-scrollbars {' +
      '::-webkit-scrollbar { width:8px; height:8px; }' +
      '::-webkit-scrollbar-thumb { background:GrayText; border-radius:4px; }' +
      '::-webkit-scrollbar-track, ::-webkit-scrollbar-corner { background:transparent; }' +
      '}');
    document.adoptedStyleSheets = [sheet].concat(document.adoptedStyleSheets);
  }
  if (document.documentElement) install();
  else {
    var observer = new MutationObserver(function () {
      if (!document.documentElement) return;
      observer.disconnect();
      install();
    });
    observer.observe(document, { childList: true });
  }
})();
