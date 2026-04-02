// Scan progress EventSource client
(function () {
  "use strict";

  var scanBtn = document.getElementById("scan-btn");
  var progressEl = document.getElementById("scan-progress");
  var stageEl = document.getElementById("scan-stage");
  var percentEl = document.getElementById("scan-percent");
  var barEl = document.getElementById("scan-bar");
  var messageEl = document.getElementById("scan-message");

  var eventSource = null;

  function showProgress() {
    if (progressEl) progressEl.classList.remove("hidden");
  }

  function hideProgress() {
    if (progressEl) progressEl.classList.add("hidden");
  }

  function updateProgress(data) {
    if (stageEl) stageEl.textContent = data.stage || "";
    if (percentEl) percentEl.textContent = (data.percent || 0) + "%";
    if (barEl) barEl.style.width = (data.percent || 0) + "%";
    if (messageEl) messageEl.textContent = data.message || "";
  }

  function closeEventSource() {
    if (eventSource) {
      eventSource.close();
      eventSource = null;
    }
  }

  function startEventSource() {
    closeEventSource();
    eventSource = new EventSource("/api/scan/events");

    eventSource.addEventListener("progress", function (e) {
      try {
        var data = JSON.parse(e.data);
        showProgress();
        updateProgress(data);
      } catch (_) {
        // ignore parse errors
      }
    });

    eventSource.addEventListener("complete", function (e) {
      try {
        var data = JSON.parse(e.data);
        updateProgress(data);
      } catch (_) {
        // ignore
      }
      closeEventSource();

      // Refresh results table via HTMX
      var resultsBody = document.getElementById("results-body");
      if (resultsBody && typeof htmx !== "undefined") {
        htmx.ajax("GET", "/api/results", { target: "#results-body" });
      } else {
        // Fallback: reload the page
        window.location.reload();
      }

      // Re-enable button
      if (scanBtn) {
        scanBtn.disabled = false;
        scanBtn.textContent = "Scan Now";
      }

      // Hide progress after a short delay
      setTimeout(hideProgress, 2000);
    });

    eventSource.addEventListener("error", function (e) {
      var msg = "Scan error";
      if (e.data) {
        try {
          var data = JSON.parse(e.data);
          msg = data.message || msg;
        } catch (_) {
          // ignore
        }
      }
      updateProgress({ stage: "Error", percent: 0, message: msg });
      closeEventSource();

      if (scanBtn) {
        scanBtn.disabled = false;
        scanBtn.textContent = "Scan Now";
      }

      setTimeout(hideProgress, 5000);
    });

    eventSource.addEventListener("idle", function () {
      closeEventSource();
    });

    eventSource.onerror = function () {
      closeEventSource();
      if (scanBtn) {
        scanBtn.disabled = false;
        scanBtn.textContent = "Scan Now";
      }
    };
  }

  if (scanBtn) {
    scanBtn.addEventListener("click", function () {
      scanBtn.disabled = true;
      scanBtn.textContent = "Scanning...";

      fetch("/api/scan", { method: "POST" })
        .then(function (res) {
          if (res.status === 409) {
            scanBtn.disabled = false;
            scanBtn.textContent = "Scan Now";
            alert("A scan is already running.");
            return;
          }
          if (res.ok) {
            showProgress();
            updateProgress({
              stage: "Starting",
              percent: 0,
              message: "Initializing scan...",
            });
            startEventSource();
          } else {
            scanBtn.disabled = false;
            scanBtn.textContent = "Scan Now";
            alert("Failed to start scan.");
          }
        })
        .catch(function () {
          scanBtn.disabled = false;
          scanBtn.textContent = "Scan Now";
          alert("Network error — could not start scan.");
        });
    });
  }
})();
