(() => {
  const buttons = document.querySelectorAll("[data-copy]");
  if (!buttons.length || !navigator.clipboard) return;
  buttons.forEach((btn) => {
    btn.addEventListener("click", async () => {
      const block = btn.closest(".code-block");
      const code = block && block.querySelector("code");
      if (!code) return;
      try {
        await navigator.clipboard.writeText(code.textContent || "");
        const prev = btn.textContent;
        btn.textContent = "COPIED";
        window.setTimeout(() => {
          btn.textContent = prev;
        }, 1200);
      } catch (_) {
        /* clipboard can fail without a secure context; leave the button */
      }
    });
  });
})();
