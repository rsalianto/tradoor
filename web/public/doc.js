// The docs page's one script: the instructions for an AI chat, shown and copied.
// They are kept in one file, which the Frost Script editor copies too.
(() => {
  const copy = document.getElementById('ai-copy');
  const shown = document.getElementById('ai-text');
  const note = document.getElementById('ai-note');
  if (!copy || !shown || !note) return;
  let text = '';
  fetch('/frost-script-ai.txt')
    .then((r) => (r.ok ? r.text() : Promise.reject(new Error(String(r.status)))))
    .then((t) => {
      text = t;
      shown.textContent = t;
      copy.disabled = false;
    })
    .catch(() => {
      shown.textContent = 'The instructions did not load: open /frost-script-ai.txt instead.';
    });
  copy.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(text);
      note.textContent = 'Copied. Paste them into your AI chat, then your Pine Script v6 indicator.';
    } catch {
      // No clipboard here: open the text, selected, for the reader to copy.
      shown.closest('details').open = true;
      getSelection()?.selectAllChildren(shown);
      note.textContent = 'Selected below: copy it with Ctrl+C (⌘C on a Mac).';
    }
  });
})();
