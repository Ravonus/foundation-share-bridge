//! Inline JS for the settings page — generic form controls plus the
//! gateway-probe helper. Both scripts ship as separate `<script>` tags on
//! the page, so they stay as two constants rather than one concatenation.

#[allow(clippy::needless_raw_string_hashes)]
pub const SETTINGS_CONTROLS_SCRIPT: &str = r####"
(() => {
  // Number steppers
  document.querySelectorAll(".num-stepper").forEach((wrap) => {
    const input = wrap.querySelector("input[type=number]");
    if (!input) return;
    wrap.querySelectorAll("button[data-step]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const step = Number(btn.getAttribute("data-step")) || 0;
        const rawStep = Number(input.getAttribute("step")) || 1;
        const min = input.getAttribute("min") !== null ? Number(input.getAttribute("min")) : -Infinity;
        const max = input.getAttribute("max") !== null ? Number(input.getAttribute("max")) : Infinity;
        const current = input.value.trim() === "" ? (step > 0 ? min === -Infinity ? 0 : min : 0) : Number(input.value);
        let next = current + step * rawStep;
        if (next < min) next = min;
        if (next > max) next = max;
        const decimals = rawStep < 1 ? (String(rawStep).split(".")[1] || "").length : 0;
        input.value = decimals > 0 ? next.toFixed(decimals) : String(Math.round(next));
        input.dispatchEvent(new Event("input", { bubbles: true }));
        input.dispatchEvent(new Event("change", { bubbles: true }));
      });
    });
  });

  // Password reveal
  document.querySelectorAll("[data-reveal]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const field = btn.closest(".password-field");
      if (!field) return;
      const input = field.querySelector("input");
      if (!input) return;
      const showing = input.getAttribute("type") === "text";
      input.setAttribute("type", showing ? "password" : "text");
      btn.textContent = showing ? "Show" : "Hide";
    });
  });

  // Dirty-form tracker
  const form = document.getElementById("settings-form-v2");
  const bar = document.getElementById("settings-save-bar");
  const hint = document.getElementById("settings-save-hint");
  if (form && bar && hint) {
    const initial = new FormData(form);
    const initialEntries = Array.from(initial.entries()).map(([k, v]) => `${k}=${v}`).sort().join("|");
    const measure = () => {
      const current = new FormData(form);
      const currentEntries = Array.from(current.entries()).map(([k, v]) => `${k}=${v}`).sort().join("|");
      const dirty = currentEntries !== initialEntries;
      bar.classList.toggle("is-dirty", dirty);
      hint.textContent = dirty ? "Unsaved changes" : "All changes saved.";
    };
    form.addEventListener("input", measure);
    form.addEventListener("change", measure);
    measure();
  }
})();
"####;

#[allow(clippy::needless_raw_string_hashes)]
pub const SETTINGS_GATEWAY_HELPER_SCRIPT: &str = r####"
(() => {
  const target = document.getElementById("public_gateway_base_url");
  if (!target) return;

  const hostnameInput = document.getElementById("gateway_hostname_input");
  const hostnameButton = document.getElementById("gateway_fill_hostname");
  const ipButton = document.getElementById("gateway_fill_ip");
  const previewValue = document.getElementById("gateway_helper_preview_value");

  const escapeHtml = (value) =>
    String(value ?? "").replace(/[&<>"']/g, (char) => ({
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      "\"": "&quot;",
      "'": "&#39;",
    }[char] || char));

  const updatePreview = (value) => {
    if (!previewValue) return;
    previewValue.innerHTML = escapeHtml(value || "");
  };

  const normalizeHost = (value) => {
    const trimmed = String(value ?? "").trim();
    if (!trimmed) return "";
    const withoutScheme = trimmed.replace(/^https?:\/\//i, "");
    return withoutScheme.replace(/\/+.*$/, "").replace(/\/+$/g, "");
  };

  if (hostnameButton) {
    hostnameButton.addEventListener("click", () => {
      const host = normalizeHost(hostnameInput ? hostnameInput.value : "");
      if (!host) {
        if (hostnameInput) hostnameInput.focus();
        return;
      }
      target.value = `https://${host}`;
      updatePreview(target.value);
      target.focus();
    });
  }

  if (ipButton) {
    ipButton.addEventListener("click", () => {
      const gatewayUrl = ipButton.getAttribute("data-gateway-url");
      if (!gatewayUrl) return;
      target.value = gatewayUrl;
      updatePreview(target.value);
      target.focus();
    });
  }

  target.addEventListener("input", () => {
    updatePreview(target.value);
  });

  updatePreview(target.value);
})();
"####;
