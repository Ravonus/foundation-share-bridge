//! Inline JS powering the live-status indicator on the root dashboard.

#[allow(clippy::needless_raw_string_hashes)]
pub const LIVE_STATUS_SCRIPT: &str = r####"
(() => {
  const phaseNode = document.getElementById("live-status-phase");
  const detailNode = document.getElementById("live-status-detail");

  const refresh = async () => {
    if (!phaseNode || !detailNode) return;
    try {
      const response = await fetch("/status/live", { headers: { Accept: "application/json" } });
      if (!response.ok) return;
      const data = await response.json();
      const phase = data.phase === "idle" ? "Idle" : data.phase;
      phaseNode.className = data.phase === "idle" ? "pill" : "pill ok";
      phaseNode.textContent = phase;
      const current = data.progressCurrent;
      const total = data.progressTotal;
      const progress = current != null && total != null && total > 0 ? ` · ${current} of ${total}` : "";
      const detailText = (data.detail || (data.phase === "idle" ? "The helper is resting between cycles." : "")) + progress;
      detailNode.textContent = detailText;
    } catch (error) {
      /* swallow */
    }
  };

  const gatewayButton = document.getElementById("gateway-check-run");
  const gatewayStatus = document.getElementById("gateway-check-status");
  const localPill = document.getElementById("gateway-check-local-pill");
  const publicPill = document.getElementById("gateway-check-public-pill");
  const utilityPill = document.getElementById("gateway-check-utility-pill");

  const setPill = (node, ok) => {
    if (!node) return;
    if (ok == null) {
      node.textContent = "Idle";
      node.className = "pill";
    } else if (ok) {
      node.textContent = "Reachable";
      node.className = "pill ok";
    } else {
      node.textContent = "Unreachable";
      node.className = "pill err";
    }
  };

  if (gatewayButton) {
    gatewayButton.addEventListener("click", async () => {
      gatewayButton.disabled = true;
      if (gatewayStatus) gatewayStatus.textContent = "Probing gateways…";
      setPill(localPill, null);
      setPill(publicPill, null);
      setPill(utilityPill, null);
      try {
        const response = await fetch("/gateway/health", { headers: { Accept: "application/json" } });
        if (!response.ok) throw new Error(`Gateway check failed (${response.status})`);
        const data = await response.json();
        setPill(localPill, data.localOk ?? null);
        setPill(publicPill, data.publicOk ?? null);
        setPill(utilityPill, data.utilityOk ?? null);
        if (gatewayStatus) {
          gatewayStatus.textContent = `Checked ${new Date(data.checkedAt).toLocaleTimeString()}`;
        }
      } catch (error) {
        if (gatewayStatus) gatewayStatus.textContent = error instanceof Error ? error.message : "Gateway check failed.";
      } finally {
        gatewayButton.disabled = false;
      }
    });
  }

  void refresh();
  window.setInterval(refresh, 5000);
})();
"####;
