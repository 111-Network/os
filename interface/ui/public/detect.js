// Feature detect, before the app. Tailwind 4 output needs color-mix() and @layer (Chrome 111+, Safari 16.4+, Firefox 128+).
// On older browsers swap the whole body for a plain monochrome notice. The app entry then finds no #root and never mounts.
// External file (not inline) so the daemon's Content-Security-Policy can say script-src 'self'.
(function () {
  var ok = false;
  try {
    ok = !!(window.CSS && CSS.supports && CSS.supports("color", "color-mix(in oklab, red, blue)"));
  } catch (e) {
    ok = false;
  }
  if (ok) return;
  window.__111_unsupported = true;
  document.documentElement.style.background = "#000";
  document.body.className = "";
  document.body.setAttribute("style", "margin:0;background:#000;color:#fff");
  document.body.innerHTML =
    '<main style="max-width:40em;padding:24px 16px;font:14px/1.6 ui-monospace,Menlo,Consolas,monospace">' +
    "<p>This browser is too old for the 111 dashboard (needs Chrome 111+, Safari 16.4+ or Firefox 128+). " +
    'The minimal node page still works: <a href="/v1" style="color:#fff">/v1</a></p>' +
    "</main>";
})();
