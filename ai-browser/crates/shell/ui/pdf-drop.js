(function () {
  if (window.top !== window || !window.chrome || !window.chrome.webview ||
      typeof window.chrome.webview.postMessageWithAdditionalObjects !== 'function') return;

  function uploadTarget(event) {
    var target = event.target instanceof Element ? event.target : null;
    if (!target) return false;
    if (target.closest('input[type="file"]')) return true;
    var label = target.closest('label');
    return !!(label && label.control && label.control.type === 'file');
  }

  function fileDrag(event) {
    return event.isTrusted && !event.defaultPrevented && !uploadTarget(event) &&
      event.dataTransfer && Array.prototype.indexOf.call(event.dataTransfer.types, 'Files') !== -1;
  }

  function dragover(event) {
    if (!fileDrag(event)) return;
    var items = Array.from(event.dataTransfer.items || []);
    if (items.length && !items.some(function (item) {
      return item.kind === 'file' && (!item.type || item.type === 'application/pdf' || item.type === 'application/octet-stream');
    })) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = 'copy';
  }

  function drop(event) {
    if (!fileDrag(event)) return;
    var files = Array.from(event.dataTransfer.files || []);
    var file = files.find(function (item) { return /\.pdf$/i.test(item.name); });
    if (!file) return;
    try {
      window.chrome.webview.postMessageWithAdditionalObjects('native-browser:pdf-drop', [file]);
      event.preventDefault();
    } catch (_) {}
  }

  function install() {
    window.removeEventListener('dragover', dragover);
    window.removeEventListener('drop', drop);
    window.addEventListener('dragover', dragover);
    window.addEventListener('drop', drop);
  }
  install();
  // Page-level upload handlers installed during startup get the first chance to accept a drop.
  document.addEventListener('DOMContentLoaded', function () { setTimeout(install, 0); }, { once: true });
  window.addEventListener('load', function () { setTimeout(install, 0); }, { once: true });
})();
