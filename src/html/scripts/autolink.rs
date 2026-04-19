//! Inline JS that auto-refreshes the root page after initial autolink.

#[allow(clippy::needless_raw_string_hashes)]
pub const ROOT_AUTOLINK_SCRIPT: &str = r####"
(() => {
  const form = document.getElementById("autolink-form");
  const status = document.getElementById("autolink-status");
  if (!form) return;

  const countdownSeconds = 6;
  let remaining = countdownSeconds;
  let submitted = false;

  const submit = () => {
    if (submitted) return;
    submitted = true;
    if (status) {
      status.textContent = "Confirming with the archive site now…";
    }
    if (typeof form.requestSubmit === "function") {
      form.requestSubmit();
    } else {
      form.submit();
    }
  };

  const tick = () => {
    if (submitted) return;
    if (remaining <= 0) {
      submit();
      return;
    }
    if (status) {
      status.textContent = `Confirming pairing automatically in ${remaining} second${remaining === 1 ? "" : "s"}… press the button to do it now.`;
    }
    remaining -= 1;
    window.setTimeout(tick, 1000);
  };

  form.addEventListener("submit", () => {
    submitted = true;
  });

  tick();
})();
"####;
