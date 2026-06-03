/**
 * Shared "Reset to Default" handler for the admin prompt-edit modals
 * (templates/admin/edit_*_prompt_modal.html).
 *
 * Each reset button carries data-reset-target with a selector for the prompt
 * textarea to repopulate from the modal's hidden #default_prompt_content.
 */
(function () {
  'use strict';

  document.body.addEventListener('click', function (e) {
    const btn = e.target.closest('[data-reset-target]');
    if (!btn) return;
    const scope = btn.closest('dialog') || document;
    const source = scope.querySelector('#default_prompt_content');
    const target = scope.querySelector(btn.dataset.resetTarget);
    if (source && target) target.value = source.value;
  });
})();
