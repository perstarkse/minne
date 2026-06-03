/**
 * Modal lifecycle for markup injected into #modal (see templates/modal_base.html).
 *
 * Uses delegated listeners so we do not rely on inline <script> re-execution on
 * each hx-swap="innerHTML". Successful submit close is per-form via hx-on in the template.
 */
(function () {
  'use strict';

  function getDialog() {
    return document.getElementById('body_modal');
  }

  // Auto-open the dialog whenever new modal markup is swapped into #modal.
  document.body.addEventListener('htmx:afterSwap', function (e) {
    if (!e.detail.target || e.detail.target.id !== 'modal') return;
    const dialog = getDialog();
    if (dialog && typeof dialog.showModal === 'function' && !dialog.open) {
      dialog.showModal();
    }
  });

  // Submit success → close: hx-on::after-request on #modal_form (modal_base.html)
  // and on scratchpad ingest-form; not handled here.

  // Clear modal content on close so browser back/forward can't reopen it.
  // The dialog 'close' event does not bubble, so listen in the capture phase.
  document.body.addEventListener('close', function (e) {
    if (e.target && e.target.id === 'body_modal') {
      e.target.innerHTML = '';
    }
  }, true);
})();
