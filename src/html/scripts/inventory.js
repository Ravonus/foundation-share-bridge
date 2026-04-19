(() => {
  const browser = document.getElementById("inventory-browser");
  if (!browser) return;

  const grid = document.getElementById("inventory-grid");
  const emptyState = document.getElementById("inventory-empty");
  const loadMoreButton = document.getElementById("inventory-load-more");
  const statusNode = document.getElementById("inventory-status");
  const sentinel = document.getElementById("inventory-sentinel");
  const pageSize = Number(browser.getAttribute("data-page-size") || "12");
  const state = {
    loading: false,
    nextCursor: null,
    done: false,
    loadedAny: false,
    error: false,
  };

  const previewObserver = "IntersectionObserver" in window
    ? new IntersectionObserver((entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          const node = entry.target;
          if (!node.getAttribute("src")) {
            loadPreviewCandidate(node, 0);
          }
          previewObserver.unobserve(node);
        }
      }, { rootMargin: "220px 0px" })
    : null;

  const paginationObserver = sentinel && "IntersectionObserver" in window
    ? new IntersectionObserver((entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            void loadNextPage();
          }
        }
      }, { rootMargin: "320px 0px" })
    : null;

  if (paginationObserver && sentinel) {
    paginationObserver.observe(sentinel);
  }

  const escapeHtml = (value) =>
    String(value ?? "").replace(/[&<>"']/g, (char) => ({
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      "\"": "&quot;",
      "'": "&#39;",
    }[char] || char));

  const formatTimestamp = (value) => {
    if (!value) return "Never";
    const parsed = new Date(value);
    if (Number.isNaN(parsed.getTime())) return String(value);
    return parsed.toLocaleString(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    });
  };

  const shortAddress = (value) => {
    const text = String(value ?? "").trim();
    if (text.length <= 12) return text;
    return `${text.slice(0, 6)}…${text.slice(-4)}`;
  };

  const uniqueStrings = (values) =>
    Array.from(
      new Set(
        values
          .map((value) => String(value ?? "").trim())
          .filter((value) => value.length > 0),
      ),
    );

  const guessKindFromUrl = (value) => {
    const text = String(value ?? "").toLowerCase();
    if (!text) return "UNKNOWN";
    if (text.includes(".mp4") || text.includes(".mov") || text.includes(".webm") || text.includes("video")) {
      return "VIDEO";
    }
    if (text.includes(".mp3") || text.includes(".wav") || text.includes(".ogg") || text.includes(".aac") || text.includes("audio")) {
      return "AUDIO";
    }
    if (
      text.includes(".png") ||
      text.includes(".jpg") ||
      text.includes(".jpeg") ||
      text.includes(".gif") ||
      text.includes(".svg") ||
      text.includes(".webp") ||
      text.includes("image")
    ) {
      return "IMAGE";
    }
    if (text.includes(".html") || text.includes("text/html")) {
      return "HTML";
    }
    if (
      text.includes(".glb") ||
      text.includes(".gltf") ||
      text.includes(".usdz") ||
      text.includes("model/gltf") ||
      text.includes("model/vnd.usdz") ||
      text.includes("model")
    ) {
      return "MODEL";
    }
    return "UNKNOWN";
  };

  const normalizeKind = (value) => {
    const text = String(value ?? "").trim().toUpperCase();
    if (text === "IMAGE" || text === "VIDEO" || text === "AUDIO" || text === "HTML" || text === "MODEL") {
      return text;
    }
    return "UNKNOWN";
  };

  const stripQueryString = (value) => {
    const raw = String(value ?? "");
    const cut = raw.indexOf("?");
    return cut === -1 ? raw : raw.slice(0, cut);
  };

  const isUsdzUrl = (value) =>
    stripQueryString(value).toLowerCase().endsWith(".usdz");

  const supportsInlineModelPreview = (value) => !isUsdzUrl(value);

  const previewKindForPreviewUrl = (url, fallbackKind, openUrl) => {
    if (openUrl && url !== openUrl) return "IMAGE";

    const fromUrl = guessKindFromUrl(url);
    if (fromUrl !== "UNKNOWN") return fromUrl;
    return fallbackKind;
  };

  const getRelatedCids = (item) =>
    uniqueStrings([
      item?.cid,
      item?.metadataCid,
      item?.mediaCid,
      ...(Array.isArray(item?.relatedCids) ? item.relatedCids : []),
    ]);

  const buildPublicGatewayUrl = (item) =>
    item?.cid ? `https://dweb.link/ipfs/${encodeURIComponent(String(item.cid).trim())}` : "";

  const buildPreviewCandidates = (item) => {
    const previewEntries = [
      item?.previewLocalGatewayUrl
        ? {
            url: item.previewLocalGatewayUrl,
            kind: previewKindForPreviewUrl(
              item.previewLocalGatewayUrl,
              normalizeKind(item.mediaKind),
              item.localGatewayUrl,
            ),
          }
        : null,
      item?.previewPublicGatewayUrl
        ? {
            url: item.previewPublicGatewayUrl,
            kind: previewKindForPreviewUrl(
              item.previewPublicGatewayUrl,
              normalizeKind(item.mediaKind),
              item.publicGatewayUrl,
            ),
          }
        : null,
      item?.localGatewayUrl
        ? { url: item.localGatewayUrl, kind: normalizeKind(item.mediaKind) }
        : null,
      item?.publicGatewayUrl
        ? { url: item.publicGatewayUrl, kind: normalizeKind(item.mediaKind) }
        : null,
      buildPublicGatewayUrl(item)
        ? { url: buildPublicGatewayUrl(item), kind: normalizeKind(item.mediaKind) }
        : null,
    ].filter((value) => value && value.url);

    const seen = new Set();
    return previewEntries.filter((entry) => {
      if (seen.has(entry.url)) return false;
      seen.add(entry.url);
      return true;
    });
  };

  const choosePinnedUrl = (item) =>
    item.publicGatewayUrl || item.localGatewayUrl || buildPublicGatewayUrl(item) || "";

  const choosePublicUrl = (item) => buildPublicGatewayUrl(item);

  const buildContextHtml = (item) => {
    if (item.foundationUrl) {
      return `<a href="${escapeHtml(item.foundationUrl)}" target="_blank" rel="noreferrer">Open work page</a>`;
    }
    if (item.contractAddress && item.tokenId) {
      return `${escapeHtml(shortAddress(item.contractAddress))} #${escapeHtml(item.tokenId)}`;
    }
    if (item.username) return `@${escapeHtml(item.username)}`;
    if (item.artistUsername) return `@${escapeHtml(item.artistUsername)}`;
    if (item.sourceKind) return escapeHtml(item.sourceKind);
    return "Pinned on this computer";
  };

  const buildPreviewHtml = (item, title) => {
    const candidates = buildPreviewCandidates(item);
    const primary = candidates[0] ?? null;
    if (!primary) {
      return `<div class="pin-preview-empty">No preview URL yet for this CID.</div>`;
    }

    const encodedCandidates = escapeHtml(
      candidates.map((entry) => `${entry.kind}|${entry.url}`).join("\n"),
    );

    if (primary.kind === "IMAGE") {
      return `<img class="pin-preview-media pin-preview-loadable" alt="${escapeHtml(title)}" data-preview-candidates="${encodedCandidates}" loading="lazy" />`;
    }

    if (primary.kind === "VIDEO") {
      return `<video class="pin-preview-media pin-preview-loadable" aria-label="Preview for ${escapeHtml(title)}" data-preview-candidates="${encodedCandidates}" muted playsinline controls preload="metadata"></video>`;
    }

    if (primary.kind === "AUDIO") {
      return `<div class="pin-preview-audio"><audio class="pin-preview-loadable" aria-label="Preview for ${escapeHtml(title)}" data-preview-candidates="${encodedCandidates}" controls preload="metadata"></audio></div>`;
    }

    if (primary.kind === "MODEL") {
      const modelCandidates = candidates.filter(
        (entry) => entry.kind === "MODEL" && supportsInlineModelPreview(entry.url),
      );
      const usdzCandidate = candidates.find(
        (entry) => entry.kind === "MODEL" && isUsdzUrl(entry.url),
      );
      const posterCandidate = candidates.find((entry) => entry.kind === "IMAGE");

      if (modelCandidates.length === 0 && usdzCandidate) {
        const posterSrc = posterCandidate ? posterCandidate.url : usdzCandidate.url;
        return `<a class="pin-preview-ar" rel="ar" href="${escapeHtml(usdzCandidate.url)}"><img alt="${escapeHtml(title)}" src="${escapeHtml(posterSrc)}" /></a>`;
      }

      const inlineCandidatesEncoded = escapeHtml(
        modelCandidates.map((entry) => `${entry.kind}|${entry.url}`).join("\n"),
      );
      const iosSrcAttr = usdzCandidate
        ? ` ios-src="${escapeHtml(usdzCandidate.url)}"`
        : "";
      const posterAttr = posterCandidate
        ? ` poster="${escapeHtml(posterCandidate.url)}"`
        : "";
      return `<model-viewer class="pin-preview-model pin-preview-loadable" alt="${escapeHtml(title)}" data-preview-candidates="${inlineCandidatesEncoded}"${iosSrcAttr}${posterAttr} ar ar-modes="webxr scene-viewer quick-look" camera-controls touch-action="pan-y" interaction-prompt="none" shadow-intensity="0.85" exposure="1" environment-image="neutral"><div class="pin-preview-empty">Loading 3D preview…</div></model-viewer>`;
    }

    return `<iframe class="pin-preview-frame pin-preview-loadable" title="Preview for ${escapeHtml(title)}" data-preview-candidates="${encodedCandidates}" referrerpolicy="no-referrer" allowfullscreen></iframe>`;
  };

  const buildVerificationSummary = (item) => {
    if (!item.lastVerifiedAt) {
      return "Network visibility has not been checked yet.";
    }
    const detail = item.lastError
      ? ` · ${escapeHtml(item.lastError)}`
      : "";
    return `Last checked ${escapeHtml(formatTimestamp(item.lastVerifiedAt))}${detail}`;
  };

  const buildNoteHtml = (item) => {
    if (item.lastError) {
      return `<p class="pin-note err">${escapeHtml(item.lastError)}</p>`;
    }
    if (item.lastSyncError) {
      return `<p class="pin-note err">${escapeHtml(item.lastSyncError)}</p>`;
    }
    return "";
  };

  const formatRootsSummary = (item) => {
    const totalRoots = getRelatedCids(item).length;
    if (totalRoots <= 1) return "1 linked root";
    return `${totalRoots} linked roots`;
  };

  const buildMetadataViewerId = (item) =>
    `pin-metadata-${encodeURIComponent(String(item?.cid ?? "").trim()).replace(/[^a-zA-Z0-9_-]+/g, "")}`;

  const metadataToggleCopy = (metadataView) => {
    if (!metadataView) return "";
    const fieldCount = Array.isArray(metadataView.fields) ? metadataView.fields.length : 0;
    const attributeCount = Array.isArray(metadataView.attributes) ? metadataView.attributes.length : 0;
    const pieces = [];
    if (fieldCount > 0) pieces.push(`${fieldCount} detail${fieldCount === 1 ? "" : "s"}`);
    if (attributeCount > 0) pieces.push(`${attributeCount} trait${attributeCount === 1 ? "" : "s"}`);
    if (pieces.length === 0) return "raw JSON";
    return pieces.join(" · ");
  };

  const renderMetadataLines = (entries) =>
    entries
      .filter((entry) => entry && entry.label && entry.value)
      .map((entry) => `
        <div class="pin-metadata-line">
          <strong>${escapeHtml(entry.label)}</strong>
          <span class="pin-metadata-value">${escapeHtml(entry.value)}</span>
        </div>
      `)
      .join("");

  const renderMetadataTraits = (entries) => {
    if (!Array.isArray(entries) || entries.length === 0) return "";

    return `
      <div class="pin-metadata-traits">
        <div class="pin-metadata-json-head">
          <strong>Traits</strong>
        </div>
        <div class="pin-metadata-trait-grid">
          ${entries
            .filter((entry) => entry && entry.label && entry.value)
            .map((entry) => `
              <div class="pin-metadata-trait">
                <strong>${escapeHtml(entry.label)}</strong>
                <span>${escapeHtml(entry.value)}</span>
              </div>
            `)
            .join("")}
        </div>
      </div>
    `;
  };

  const renderMetadataViewer = (item) => {
    const metadataView = item?.metadataView;
    if (!metadataView) return "";

    const viewerId = buildMetadataViewerId(item);
    const detailEntries = [
      item?.metadataCid ? { label: "Metadata CID", value: item.metadataCid } : null,
      item?.mediaCid ? { label: "Media CID", value: item.mediaCid } : null,
      ...(Array.isArray(metadataView.fields) ? metadataView.fields : []),
    ].filter(Boolean);

    const description = metadataView.description
      ? `<p class="pin-metadata-description">${escapeHtml(metadataView.description)}</p>`
      : "";
    const detailLines = detailEntries.length
      ? `<div class="pin-metadata-lines">${renderMetadataLines(detailEntries)}</div>`
      : "";
    const traits = renderMetadataTraits(metadataView.attributes);
    const rawJson = metadataView.rawJson
      ? `
        <div class="pin-metadata-json-wrap">
          <div class="pin-metadata-json-head">
            <strong>Raw JSON</strong>
            ${
              metadataView.rawJsonTruncated
                ? '<span class="pin-metadata-json-note">trimmed for speed</span>'
                : ""
            }
          </div>
          <pre class="pin-metadata-json"><code>${escapeHtml(metadataView.rawJson)}</code></pre>
        </div>
      `
      : "";

    return `
      <div class="pin-metadata-inline">
        <button
          type="button"
          class="btn ghost pin-meta-toggle"
          data-toggle-metadata
          data-metadata-target="${escapeHtml(viewerId)}"
          data-open-label="Hide metadata"
          data-closed-label="Show metadata"
          aria-expanded="false"
          aria-controls="${escapeHtml(viewerId)}"
        >
          <span data-toggle-label>Show metadata</span>
          <span class="pin-meta-toggle-copy">${escapeHtml(metadataToggleCopy(metadataView))}</span>
        </button>
        <div class="pin-metadata-viewer" id="${escapeHtml(viewerId)}" aria-hidden="true">
          <div class="pin-metadata-viewer-inner">
            <div class="pin-metadata-panel">
              ${description}
              ${detailLines}
              ${traits}
              ${rawJson}
            </div>
          </div>
        </div>
      </div>
    `;
  };

  const buildProviderBadge = (item) => {
    const count = typeof item.providerCount === "number" ? item.providerCount : null;
    if (count == null) return "";
    if (count === 0) {
      return `<span class="pill err" title="No peers know about this CID">0 peers</span>`;
    }
    if (count <= 2) {
      return `<span class="pill warn" title="Thinly replicated">${count} peer${count === 1 ? "" : "s"}</span>`;
    }
    return `<span class="pill ok" title="Well-seeded">${count} peers</span>`;
  };

  const buildRetryBadge = (item) => {
    const attempts = Number(item.retryAttempts) || 0;
    if (attempts <= 0) return "";
    const when = item.nextRetryAt ? ` (next ${formatTimestamp(item.nextRetryAt)})` : "";
    return `<span class="pill warn" title="Attempts so far">Retry ${attempts}${when}</span>`;
  };

  const buildRemoteBadge = (item) => {
    if (!item.remotePinned) return "";
    const label = item.remotePinService ? `Remote · ${escapeHtml(item.remotePinService)}` : "Remote backup";
    return `<span class="pill ok" title="Mirrored on a remote pinning service">${label}</span>`;
  };

  const buildTagsRow = (item) => {
    const tags = Array.isArray(item.customTags) ? item.customTags.filter(Boolean) : [];
    if (tags.length === 0) return "";
    return `<div class="btn-row" style="gap:6px;">${
      tags.map((t) => `<span class="pill" title="Tag">${escapeHtml(t)}</span>`).join("")
    }</div>`;
  };

  const buildErrorHintHtml = (item) => {
    if (!item.errorCategory) return "";
    const labels = {
      daemon_unreachable: "IPFS daemon unreachable",
      timeout: "Network timeout",
      no_providers: "No peers have this CID",
      not_pinned: "Not pinned locally",
      invalid_cid: "Invalid CID",
      unauthorized: "IPFS API rejected the request",
      disk_full: "Datastore is full",
      unknown: "Unknown error",
    };
    const label = labels[item.errorCategory] || item.errorCategory;
    return `<p class="pin-note" data-error-category="${escapeHtml(item.errorCategory)}"><strong>${escapeHtml(label)}</strong></p>`;
  };

  const renderCard = (item) => {
    const title = item.title || item.label || "Local IPFS pin";
    const statusLabel = item.pinned ? (item.pinType || "pinned") : "repair needed";
    const statusClass = item.pinned ? "ok" : "warn";
    const pinnedUrl = choosePinnedUrl(item);
    const publicUrl = choosePublicUrl(item);
    const localUrl = item.localGatewayUrl || "";
    const relatedCids = getRelatedCids(item);
    const syncedValue = item.syncPath
      ? escapeHtml(item.syncPath)
      : "Not synced to disk";
    const providerBadge = buildProviderBadge(item);
    const retryBadge = buildRetryBadge(item);
    const remoteBadge = buildRemoteBadge(item);
    const tagsRow = buildTagsRow(item);
    const hintBlock = buildErrorHintHtml(item);

    return `
      <article class="pin-card" data-cid="${escapeHtml(item.cid)}" data-related-cids="${escapeHtml(relatedCids.join(","))}">
        <div class="pin-preview">
          ${buildPreviewHtml(item, title)}
        </div>
        <div class="pin-card-body">
          <div class="pin-card-head">
            <div>
              <p class="pin-title">${escapeHtml(title)}</p>
              <p class="cid">${escapeHtml(item.cid)}</p>
            </div>
            <div class="btn-row" style="justify-content:flex-end; gap:6px; margin:0;">
              <span class="pill ${statusClass}">${escapeHtml(statusLabel)}</span>
              ${providerBadge}
              ${retryBadge}
              ${remoteBadge}
            </div>
          </div>

          <p class="pin-context">${buildContextHtml(item)}</p>

          ${tagsRow}

          <div class="pin-meta">
            <div class="pin-meta-line">
              <strong>Source</strong>
              <span>${escapeHtml(item.label || item.sourceKind || "watched pin")}</span>
            </div>
            <div class="pin-meta-line">
              <strong>Verified</strong>
              <span>${escapeHtml(formatTimestamp(item.lastVerifiedAt))}</span>
            </div>
            <div class="pin-meta-line">
              <strong>Synced</strong>
              <span>${syncedValue}</span>
            </div>
            <div class="pin-meta-line">
              <strong>Roots</strong>
              <span>${escapeHtml(formatRootsSummary(item))}</span>
            </div>
          </div>

          ${hintBlock}
          ${buildNoteHtml(item)}

          <div class="pin-actions">
            <button type="button" class="btn ghost" data-verify-cids="${escapeHtml(relatedCids.join(","))}">Test on network</button>
            <button type="button" class="btn ghost" data-diagnose-cid="${escapeHtml(item.cid)}">Diagnose</button>
            <button type="button" class="btn ghost" data-retry-cid="${escapeHtml(item.cid)}">Retry now</button>
            ${item.lastSyncError ? `<button type="button" class="btn ghost" data-retry-sync-cid="${escapeHtml(item.cid)}">Retry sync</button>` : ""}
            <button type="button" class="btn ghost" data-tag-cid="${escapeHtml(item.cid)}">Tags</button>
            <button type="button" class="btn ghost" data-unwatch-cids="${escapeHtml(relatedCids.join(","))}">Stop repairing</button>
            ${pinnedUrl ? `<a class="btn" href="${escapeHtml(pinnedUrl)}" target="_blank" rel="noreferrer">Open pinned</a>` : ""}
            ${publicUrl ? `<a class="btn ghost" href="${escapeHtml(publicUrl)}" target="_blank" rel="noreferrer">Open public</a>` : ""}
            ${localUrl ? `<a class="btn ghost" href="${escapeHtml(localUrl)}" target="_blank" rel="noreferrer">Open local</a>` : ""}
            ${renderMetadataViewer(item)}
          </div>

          <p class="pin-test-status">${buildVerificationSummary(item)}</p>
        </div>
      </article>
    `;
  };

  const readPreviewCandidates = (node) =>
    String(node.getAttribute("data-preview-candidates") || "")
      .split("\n")
      .map((entry) => entry.trim())
      .filter(Boolean)
      .map((entry) => {
        const divider = entry.indexOf("|");
        if (divider === -1) return { kind: "UNKNOWN", url: entry };
        return {
          kind: entry.slice(0, divider) || "UNKNOWN",
          url: entry.slice(divider + 1),
        };
      })
      .filter((entry) => entry.url);

  const loadPreviewCandidate = (node, index) => {
    const candidates = readPreviewCandidates(node);
    const next = candidates[index] ?? null;
    if (!next) return false;

    if (
      node.tagName === "IMG" ||
      node.tagName === "IFRAME" ||
      node.tagName === "VIDEO" ||
      node.tagName === "AUDIO" ||
      node.tagName === "MODEL-VIEWER"
    ) {
      node.setAttribute("src", next.url);
    }
    if ((node.tagName === "VIDEO" || node.tagName === "AUDIO") && typeof node.load === "function") {
      node.load();
    }

    node.setAttribute("data-preview-index", String(index));
    return true;
  };

  const advancePreviewCandidate = (node) => {
    const currentIndex = Number(node.getAttribute("data-preview-index") || "0");
    return loadPreviewCandidate(node, currentIndex + 1);
  };

  const hydratePreviewMedia = () => {
    const nodes = grid.querySelectorAll(".pin-preview-loadable[data-preview-candidates]");
    for (const node of nodes) {
      if (node.getAttribute("src")) continue;

      if (!node.hasAttribute("data-preview-error-bound")) {
        node.setAttribute("data-preview-error-bound", "true");
        node.addEventListener("error", () => {
          const advanced = advancePreviewCandidate(node);
          if (!advanced) {
            const container = node.closest(".pin-preview");
            if (container) {
              container.innerHTML = `<div class="pin-preview-empty">Preview unavailable right now.</div>`;
            }
          }
        });
      }

      if (!previewObserver) {
        loadPreviewCandidate(node, 0);
        continue;
      }
      previewObserver.observe(node);
    }
  };

  const setStatus = (message) => {
    if (statusNode) {
      statusNode.textContent = message;
    }
  };

  const syncControls = () => {
    if (!loadMoreButton) return;
    loadMoreButton.disabled = state.loading;
    loadMoreButton.hidden = !state.nextCursor && !state.error;
    loadMoreButton.textContent = state.error ? "Retry load" : "Load more works";
  };

  const loadNextPage = async () => {
    if (state.loading || (state.done && !state.error)) return;

    state.loading = true;
    state.error = false;
    syncControls();
    setStatus(state.loadedAny ? "Loading more works…" : "Loading saved works…");

    try {
      const url = new URL("/pins/page", window.location.origin);
      url.searchParams.set("limit", String(pageSize));
      if (state.nextCursor) {
        url.searchParams.set("cursor", state.nextCursor);
      }

      const response = await fetch(url.toString(), {
        headers: { Accept: "application/json" },
      });
      if (!response.ok) {
        throw new Error(`Inventory request failed (${response.status})`);
      }

      const payload = await response.json();
      const items = Array.isArray(payload.items) ? payload.items : [];

      if (items.length > 0) {
        grid.insertAdjacentHTML("beforeend", items.map(renderCard).join(""));
        hydratePreviewMedia();
        state.loadedAny = true;
        if (emptyState) emptyState.hidden = true;
      } else if (!state.loadedAny && emptyState) {
        emptyState.hidden = false;
      }

      state.nextCursor = payload.nextCursor || null;
      state.done = !state.nextCursor;
      state.error = false;
      syncControls();

      if (state.done) {
        setStatus(state.loadedAny ? `Showing ${grid.children.length} works.` : "No saved works available.");
      } else {
        setStatus(`Showing ${grid.children.length} of ${payload.total} works.`);
      }
    } catch (error) {
      state.done = true;
      state.error = true;
      syncControls();
      setStatus(error instanceof Error ? error.message : "Unable to load saved works.");
    } finally {
      state.loading = false;
      syncControls();
    }
  };

  const toggleMetadataViewer = (button) => {
    const targetId = String(button.getAttribute("data-metadata-target") || "").trim();
    if (!targetId) return;

    const viewer = document.getElementById(targetId);
    if (!viewer) return;

    const isOpen = !viewer.classList.contains("is-open");
    viewer.classList.toggle("is-open", isOpen);
    viewer.setAttribute("aria-hidden", String(!isOpen));
    button.setAttribute("aria-expanded", String(isOpen));

    const labelNode = button.querySelector("[data-toggle-label]");
    const nextLabel = isOpen
      ? button.getAttribute("data-open-label")
      : button.getAttribute("data-closed-label");
    if (labelNode && nextLabel) {
      labelNode.textContent = nextLabel;
    }
  };

  browser.addEventListener("click", async (event) => {
    const metadataButton = event.target.closest("[data-toggle-metadata]");
    if (metadataButton) {
      toggleMetadataViewer(metadataButton);
      return;
    }

    const unwatchButton = event.target.closest("[data-unwatch-cids]");
    if (unwatchButton) {
      const cids = uniqueStrings(
        String(unwatchButton.getAttribute("data-unwatch-cids") || "").split(","),
      );
      const card = unwatchButton.closest(".pin-card");
      const resultNode = card ? card.querySelector(".pin-test-status") : null;
      if (cids.length === 0 || !resultNode) return;

      const confirmed = window.confirm(
        `Stop repairing ${cids.length} linked root${cids.length === 1 ? "" : "s"} for this saved work? Existing IPFS pins and synced files will be left alone.`,
      );
      if (!confirmed) return;

      unwatchButton.setAttribute("disabled", "disabled");
      resultNode.textContent = "Removing these roots from the forever-watch list…";

      try {
        const response = await fetch("/pins/unwatch", {
          method: "POST",
          headers: {
            Accept: "application/json",
            "Content-Type": "application/json",
          },
          body: JSON.stringify({ cids }),
        });
        if (!response.ok) {
          throw new Error(`Unable to stop repairing (${response.status})`);
        }

        window.location.reload();
      } catch (error) {
        resultNode.textContent = error instanceof Error ? error.message : "Unable to stop repairing this work right now.";
      } finally {
        unwatchButton.removeAttribute("disabled");
      }
      return;
    }

    const diagnoseButton = event.target.closest("[data-diagnose-cid]");
    if (diagnoseButton) {
      const cid = diagnoseButton.getAttribute("data-diagnose-cid");
      const card = diagnoseButton.closest(".pin-card");
      const resultNode = card ? card.querySelector(".pin-test-status") : null;
      if (!cid || !resultNode) return;
      diagnoseButton.setAttribute("disabled", "disabled");
      resultNode.textContent = "Diagnosing…";
      try {
        const response = await fetch(`/pins/item/${encodeURIComponent(cid)}/diagnose`, {
          method: "POST",
          headers: { Accept: "application/json" },
        });
        if (!response.ok) throw new Error(`Diagnose failed (${response.status})`);
        const data = await response.json();
        const parts = [];
        parts.push(data.pinnedLocally ? "Pinned locally." : "Not pinned locally.");
        parts.push(`${data.providerCount} peer${data.providerCount === 1 ? "" : "s"} on DHT.`);
        if (data.gatewayLocalOk != null) parts.push(`Local gateway ${data.gatewayLocalOk ? "reachable" : "unreachable"}.`);
        if (data.gatewayPublicOk != null) parts.push(`External gateway ${data.gatewayPublicOk ? "reachable" : "unreachable"}.`);
        if (data.errorHint) parts.push(data.errorHint);
        resultNode.textContent = parts.join(" ");
      } catch (error) {
        resultNode.textContent = error instanceof Error ? error.message : "Diagnose failed.";
      } finally {
        diagnoseButton.removeAttribute("disabled");
      }
      return;
    }

    const retryButton = event.target.closest("[data-retry-cid]");
    if (retryButton) {
      const cid = retryButton.getAttribute("data-retry-cid");
      const card = retryButton.closest(".pin-card");
      const resultNode = card ? card.querySelector(".pin-test-status") : null;
      if (!cid || !resultNode) return;
      retryButton.setAttribute("disabled", "disabled");
      resultNode.textContent = "Retrying now…";
      try {
        const response = await fetch(`/pins/item/${encodeURIComponent(cid)}/retry`, {
          method: "POST",
          headers: { Accept: "application/json" },
        });
        if (!response.ok) throw new Error(`Retry failed (${response.status})`);
        const data = await response.json();
        resultNode.textContent = data.message || (data.pinned ? "Re-pinned." : "Retry did not succeed.");
      } catch (error) {
        resultNode.textContent = error instanceof Error ? error.message : "Retry failed.";
      } finally {
        retryButton.removeAttribute("disabled");
      }
      return;
    }

    const retrySyncButton = event.target.closest("[data-retry-sync-cid]");
    if (retrySyncButton) {
      const cid = retrySyncButton.getAttribute("data-retry-sync-cid");
      const card = retrySyncButton.closest(".pin-card");
      const resultNode = card ? card.querySelector(".pin-test-status") : null;
      if (!cid || !resultNode) return;
      retrySyncButton.setAttribute("disabled", "disabled");
      resultNode.textContent = "Re-syncing to disk…";
      try {
        const response = await fetch(`/pins/item/${encodeURIComponent(cid)}/retry-sync`, {
          method: "POST",
          headers: { Accept: "application/json" },
        });
        if (!response.ok) throw new Error(`Retry-sync failed (${response.status})`);
        const data = await response.json();
        resultNode.textContent = data.synced ? `Synced to ${data.path}` : (data.error || "Retry sync did not succeed.");
      } catch (error) {
        resultNode.textContent = error instanceof Error ? error.message : "Retry sync failed.";
      } finally {
        retrySyncButton.removeAttribute("disabled");
      }
      return;
    }

    const tagButton = event.target.closest("[data-tag-cid]");
    if (tagButton) {
      const cid = tagButton.getAttribute("data-tag-cid");
      const card = tagButton.closest(".pin-card");
      const resultNode = card ? card.querySelector(".pin-test-status") : null;
      if (!cid || !resultNode) return;
      const current = Array.from(card.querySelectorAll('[data-tag-chip]')).map((n) => n.textContent || "").join(", ");
      const input = window.prompt("Comma-separated tags (leave empty to clear):", current);
      if (input == null) return;
      const tags = input.split(",").map((t) => t.trim()).filter(Boolean);
      tagButton.setAttribute("disabled", "disabled");
      resultNode.textContent = "Saving tags…";
      try {
        const response = await fetch(`/pins/item/${encodeURIComponent(cid)}/tags`, {
          method: "POST",
          headers: { Accept: "application/json", "Content-Type": "application/json" },
          body: JSON.stringify({ tags }),
        });
        if (!response.ok) throw new Error(`Tag save failed (${response.status})`);
        const data = await response.json();
        resultNode.textContent = `Tags updated · ${data.tags.join(", ") || "none"}`;
      } catch (error) {
        resultNode.textContent = error instanceof Error ? error.message : "Tag save failed.";
      } finally {
        tagButton.removeAttribute("disabled");
      }
      return;
    }

    const button = event.target.closest("[data-verify-cids]");
    if (!button) return;

    const cids = uniqueStrings(
      String(button.getAttribute("data-verify-cids") || "").split(","),
    );
    const card = button.closest(".pin-card");
    const resultNode = card ? card.querySelector(".pin-test-status") : null;
    if (cids.length === 0 || !resultNode) return;

    button.setAttribute("disabled", "disabled");
    resultNode.textContent = `Checking ${cids.length} linked root${cids.length === 1 ? "" : "s"} on the network…`;

    try {
      const response = await fetch("/pins/verify", {
        method: "POST",
        headers: {
          Accept: "application/json",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ cids }),
      });

      if (!response.ok) {
        throw new Error(`Verification failed (${response.status})`);
      }

      const payload = await response.json();
      const results = Array.isArray(payload.results) ? payload.results : [];
      const visible = results.filter(
        (entry) => entry && entry.reachable && entry.providerCount > 0,
      );
      const checkedAt =
        payload.checkedAt ||
        results
          .map((entry) => entry?.checkedAt)
          .filter(Boolean)
          .sort()
          .at(-1) ||
        null;
      const firstError =
        results.find((entry) => entry?.error)?.error ||
        null;

      if (visible.length === results.length && visible.length > 0) {
        const providerCount = Math.min(
          ...visible.map((entry) => Number(entry.providerCount) || 0),
        );
        resultNode.textContent = `Visible on the network for all ${visible.length} linked root${visible.length === 1 ? "" : "s"} via at least ${providerCount} provider${providerCount === 1 ? "" : "s"} · checked ${formatTimestamp(checkedAt)}`;
      } else if (visible.length > 0) {
        resultNode.textContent = `Only ${visible.length} of ${results.length} linked roots are visible on the network yet${firstError ? ` · ${firstError}` : ""}${checkedAt ? ` · checked ${formatTimestamp(checkedAt)}` : ""}`;
      } else if (firstError) {
        resultNode.textContent = firstError;
      } else {
        resultNode.textContent = `No linked roots are visible on the network yet${checkedAt ? ` · checked ${formatTimestamp(checkedAt)}` : ""}`;
      }
    } catch (error) {
      resultNode.textContent = error instanceof Error ? error.message : "Unable to verify this pin right now.";
    } finally {
      button.removeAttribute("disabled");
    }
  });

  if (loadMoreButton) {
    loadMoreButton.addEventListener("click", () => {
      void loadNextPage();
    });
  }

  void loadNextPage();
})();
