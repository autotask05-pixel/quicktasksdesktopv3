const { invoke } = window.__TAURI__.core;
const dialog = window.__TAURI__.dialog;

const statusText = document.querySelector("#statusText");
const statusPanel = document.querySelector("#statusPanel");
const agentInput = document.querySelector("#agentInput");
const queryInput = document.querySelector("#queryInput");
const runButton = document.querySelector("#runButton");
const clearButton = document.querySelector("#clearButton");
const resultOutput = document.querySelector("#resultOutput");
const logOutput = document.querySelector("#logOutput");
const reloadModelsButton = document.querySelector("#reloadModelsButton");
const modelPathInputs = document.querySelectorAll("[data-model-path-target]");
const modelSelectButtons = document.querySelectorAll("[data-model-select-target]");

let logCursor = null;

const ELEVATION_REQUIRED = "ELEVATION_REQUIRED";

/* =========================================================
   SHORTCUT KEYS
========================================================= */
/*
Ctrl/Cmd + Enter  -> Run query
Ctrl/Cmd + L      -> Clear results
Ctrl/Cmd + R      -> Reload models
Ctrl/Cmd + O      -> Open agent file picker
Ctrl/Cmd + Shift + M -> Select first model file
Esc               -> Clear status
Ctrl/Cmd + K      -> Focus query input
*/

function registerShortcuts() {
  document.addEventListener("keydown", async (event) => {
    const key = event.key.toLowerCase();
    const ctrl = event.ctrlKey || event.metaKey;

    // Prevent browser default shortcuts
    const prevent = () => {
      event.preventDefault();
      event.stopPropagation();
    };

    // Run query
    if (ctrl && key === "enter") {
      prevent();
      if (!runButton.disabled) {
        await runQuery();
      }
      return;
    }

    // Clear output
    if (ctrl && key === "l") {
      prevent();
      clearAllOutputs();
      return;
    }

    // Reload models
    if (ctrl && key === "r") {
      prevent();
      if (!reloadModelsButton.disabled) {
        await reloadModels();
      }
      return;
    }

    // Open agent file picker
    if (ctrl && key === "o") {
      prevent();
      if (!agentInput.disabled) {
        agentInput.click();
      }
      return;
    }

    // Select first model file
    if (ctrl && event.shiftKey && key === "m") {
      prevent();
      const firstModelSelectButton = modelSelectButtons[0];
      if (firstModelSelectButton && !firstModelSelectButton.disabled) {
        await selectModelAssetPath(firstModelSelectButton.dataset.modelSelectTarget);
      }
      return;
    }

    // Focus query input
    if (ctrl && key === "k") {
      prevent();
      queryInput.focus();
      queryInput.select();
      return;
    }

    // Escape clears status
    if (key === "escape") {
      clearStatus();
      return;
    }
  });
}

/* =========================================================
   UI HELPERS
========================================================= */

function setBusy(isBusy) {
  runButton.disabled = isBusy;
  reloadModelsButton.disabled = isBusy;
  agentInput.disabled = isBusy;

  modelPathInputs.forEach((input) => {
    input.disabled = isBusy;
  });
  modelSelectButtons.forEach((button) => {
    button.disabled = isBusy;
  });

  document.body.classList.toggle("busy", isBusy);
}

function showStatus(message, isError = false) {
  statusText.textContent = message;

  statusPanel.hidden = !isError;
  statusPanel.textContent = isError ? message : "";

  statusText.dataset.error = String(isError);
}

function clearStatus() {
  statusText.textContent = "";
  statusPanel.hidden = true;
  statusPanel.textContent = "";
}

function renderResult(value) {
  resultOutput.textContent = JSON.stringify(value, null, 2);
}

function clearAllOutputs() {
  resultOutput.textContent = "";
  logOutput.textContent = "";
  clearStatus();
}

function appendLogs(lines) {
  if (!lines.length) {
    return;
  }

  logOutput.textContent += `${
    logOutput.textContent ? "\n" : ""
  }${lines.join("\n")}`;

  logOutput.scrollTop = logOutput.scrollHeight;
}

function isElevationRequired(error) {
  return String(error).includes(ELEVATION_REQUIRED);
}

function requestElevationPassword() {
  return window.prompt(
    "This action needs elevated local command permissions.\n\nEnter your sudo password:"
  );
}

/* =========================================================
   STATUS
========================================================= */

async function refreshStatus() {
  try {
    const status = await invoke("init_status");

    if (status.initialized) {
      showStatus("Local engine ready");
    } else if (status.error) {
      showStatus(status.error, true);
    } else {
      showStatus("Load an agent JSON to initialize the local engine");
    }
  } catch (error) {
    showStatus(String(error), true);
  }
}

/* =========================================================
   LOAD AGENT
========================================================= */

async function loadAgentFile(file) {
  setBusy(true);

  try {
    const config = JSON.parse(await file.text());

    let response;

    try {
      response = await invoke("load_agent", {
        config,
        elevatedPassword: null
      });
    } catch (error) {
      if (!isElevationRequired(error)) {
        throw error;
      }

      const elevatedPassword = requestElevationPassword();

      if (!elevatedPassword) {
        throw new Error("Elevated permission was cancelled");
      }

      response = await invoke("load_agent", {
        config,
        elevatedPassword
      });
    }

    renderResult(response);
    showStatus("Local engine ready");
  } catch (error) {
    showStatus(String(error), true);
    renderResult({ error: String(error) });
  } finally {
    setBusy(false);
    agentInput.value = "";
  }
}

/* =========================================================
   QUERY EXECUTION
========================================================= */

async function runQuery() {
  const query = queryInput.value.trim();

  if (!query) {
    showStatus("Enter a query before running", true);
    queryInput.focus();
    return;
  }

  setBusy(true);

  try {
    const payload = {
      req: {
        query,
        params: {},
        files: [],
        elevatedPassword: null
      }
    };

    let response;

    try {
      response = await invoke("query", payload);
    } catch (error) {
      if (!isElevationRequired(error)) {
        throw error;
      }

      const elevatedPassword = requestElevationPassword();

      if (!elevatedPassword) {
        throw new Error("Elevated permission was cancelled");
      }

      response = await invoke("query", {
        req: {
          ...payload.req,
          elevatedPassword
        }
      });
    }

    renderResult(response);
    showStatus("Query completed");
  } catch (error) {
    showStatus(String(error), true);
    renderResult({
      error: String(error)
    });
  } finally {
    setBusy(false);
  }
}

/* =========================================================
   MODEL MANAGEMENT
========================================================= */

async function useModelAssetPath(target) {
  const input = document.querySelector(`[data-model-path-target="${target}"]`);
  const path = input?.value.trim();

  if (!path) {
    showStatus("Enter a local model file path before using it", true);
    input?.focus();
    return;
  }

  setBusy(true);

  try {
    const response = await invoke("use_model_file_path", {
      target,
      path
    });

    renderResult(response);
    showStatus(`${target} is using ${response.path}`);
  } catch (error) {
    showStatus(String(error), true);

    renderResult({
      error: String(error)
    });
  } finally {
    setBusy(false);
  }
}

function modelDialogOptions(target) {
  const isTokenizer = target.endsWith("_tokenizer");

  return {
    multiple: false,
    directory: false,
    title: isTokenizer ? "Select tokenizer JSON" : "Select ONNX model",
    filters: [
      isTokenizer
        ? {
            name: "Tokenizer JSON",
            extensions: ["json"]
          }
        : {
            name: "ONNX model",
            extensions: ["onnx"]
          }
    ]
  };
}

function normalizeSelectedPath(selected) {
  if (!selected) {
    return null;
  }

  if (Array.isArray(selected)) {
    return selected[0] ? String(selected[0]) : null;
  }

  return String(selected);
}

async function selectModelAssetPath(target) {
  if (!dialog?.open) {
    showStatus("Native file dialog is unavailable in this app build", true);
    return;
  }

  setBusy(true);

  try {
    const selected = await dialog.open(modelDialogOptions(target));
    const path = normalizeSelectedPath(selected);

    if (!path) {
      showStatus("Model file selection cancelled");
      return;
    }

    const input = document.querySelector(`[data-model-path-target="${target}"]`);
    if (input) {
      input.value = path;
      input.title = path;
    }

    const response = await invoke("use_model_file_path", {
      target,
      path
    });

    renderResult(response);
    showStatus(`${target} is using ${response.path}`);
  } catch (error) {
    showStatus(String(error), true);
    renderResult({
      error: String(error)
    });
  } finally {
    setBusy(false);
  }
}

async function reloadModels() {
  setBusy(true);

  try {
    const response = await invoke("reload_models");

    renderResult(response);
    showStatus("Models reloaded");
  } catch (error) {
    showStatus(String(error), true);
    renderResult({
      error: String(error)
    });
  } finally {
    setBusy(false);
  }
}

/* =========================================================
   LOG STREAM
========================================================= */

async function refreshLogs() {
  try {
    const response = await invoke("logs", {
      since: logCursor
    });

    logCursor = response.next_cursor;

    if (response.entries.length) {
      appendLogs(response.entries.map((entry) => entry.line));
    }
  } catch {
    // Log polling should never interrupt the app.
  }
}

/* =========================================================
   EVENT LISTENERS
========================================================= */

runButton.addEventListener("click", runQuery);

reloadModelsButton.addEventListener("click", reloadModels);

clearButton.addEventListener("click", clearAllOutputs);

agentInput.addEventListener("change", () => {
  const [file] = agentInput.files;

  if (file) {
    loadAgentFile(file);
  }
});

modelSelectButtons.forEach((button) => {
  button.addEventListener("click", () => {
    selectModelAssetPath(button.dataset.modelSelectTarget);
  });
});

modelPathInputs.forEach((input) => {
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      useModelAssetPath(input.dataset.modelPathTarget);
    }
  });
});

/* =========================================================
   INITIALIZATION
========================================================= */

registerShortcuts();

refreshStatus();
refreshLogs();

setInterval(refreshLogs, 1200);
