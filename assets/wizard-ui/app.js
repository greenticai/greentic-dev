(function () {
  "use strict";

  var app = document.getElementById("app");
  var state = {
    phase: "launcher",
    wizardType: null,
    steps: [],
    currentStep: 0,
    answers: {},
  };

  function render() {
    switch (state.phase) {
      case "launcher":
        renderLauncher();
        break;
      case "form":
        renderFormStep();
        break;
      case "review":
        renderReview();
        break;
      case "executing":
        renderExecuting();
        break;
      case "result":
        renderResult();
        break;
    }
  }

  // -- Launcher --

  function renderLauncher() {
    fetch("/api/launcher/options")
      .then(function (r) { return r.json(); })
      .then(function (data) {
        var html = '<header><h1>' + esc(data.title) + '</h1></header><main><div class="option-grid">';
        data.options.forEach(function (opt) {
          html += '<button class="option-btn" data-action="' + esc(opt.value) + '">' + esc(opt.label) + '</button>';
        });
        html += '<button class="option-btn option-exit" data-action="exit">Exit</button>';
        html += '</div></main>';
        app.innerHTML = html;
        app.querySelectorAll("[data-action]").forEach(function (btn) {
          btn.addEventListener("click", function () {
            var action = btn.getAttribute("data-action");
            if (action === "exit") {
              fetch("/api/shutdown", { method: "POST" });
              app.innerHTML = '<div class="center-msg">Wizard closed.</div>';
            } else {
              selectWizard(action);
            }
          });
        });
      });
  }

  function selectWizard(action) {
    state.wizardType = action;
    state.answers = {};
    state.currentStep = 0;
    fetch("/api/launcher/select", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ selected_action: action }),
    }).then(function () {
      return fetch("/api/wizard/steps");
    }).then(function (r) { return r.json(); })
      .then(function (data) {
        if (data && data.steps) {
          state.steps = data.steps;
          state.phase = "form";
          render();
        }
      });
  }

  // -- Multi-step Form --

  function renderFormStep() {
    var step = state.steps[state.currentStep];
    if (!step) return;

    var progress = (state.currentStep + 1) + " / " + (state.steps.length + 1);
    var html = '<header><h1>' + esc(step.title) + '</h1>';
    html += '<div class="progress">Step ' + progress + '</div></header>';
    html += '<main><p class="step-desc">' + esc(step.description) + '</p>';
    html += '<form id="step-form" class="wizard-form">';

    step.fields.forEach(function (field) {
      var depAttr = "";
      if (field.depends_on) {
        depAttr = ' data-depends-field="' + esc(field.depends_on.field) + '" data-depends-value="' + esc(field.depends_on.value) + '"';
      }
      html += '<div class="form-group"' + depAttr + '>';
      html += '<label for="f-' + esc(field.id) + '">' + esc(field.label);
      if (field.required) html += ' <span class="req">*</span>';
      html += '</label>';

      var val = state.answers[field.id] !== undefined ? state.answers[field.id] : (field.default_value || "");

      if (field.kind === "text") {
        html += '<input type="text" id="f-' + esc(field.id) + '" name="' + esc(field.id) + '" value="' + esc(String(val)) + '"';
        if (field.placeholder) html += ' placeholder="' + esc(field.placeholder) + '"';
        html += ' />';
      } else if (field.kind === "select") {
        html += '<select id="f-' + esc(field.id) + '" name="' + esc(field.id) + '">';
        field.choices.forEach(function (c) {
          var sel = String(val) === c.value ? " selected" : "";
          html += '<option value="' + esc(c.value) + '"' + sel + '>' + esc(c.label) + '</option>';
        });
        html += '</select>';
      } else if (field.kind === "boolean") {
        var checked = val === "true" || val === true ? " checked" : "";
        html += '<label class="toggle"><input type="checkbox" id="f-' + esc(field.id) + '" name="' + esc(field.id) + '"' + checked + ' /><span class="toggle-label">' + (checked ? "Yes" : "No") + '</span></label>';
      }
      html += '</div>';
    });

    html += '</form><div class="nav-buttons">';
    if (state.currentStep > 0) {
      html += '<button class="btn btn-secondary" id="btn-back">Back</button>';
    } else {
      html += '<button class="btn btn-secondary" id="btn-back-launcher">Back to menu</button>';
    }
    html += '<button class="btn btn-primary" id="btn-next">Next</button>';
    html += '</div></main>';

    app.innerHTML = html;
    setupFormListeners();
  }

  function setupFormListeners() {
    // Toggle label update
    app.querySelectorAll('input[type="checkbox"]').forEach(function (cb) {
      cb.addEventListener("change", function () {
        var lbl = cb.parentElement.querySelector(".toggle-label");
        if (lbl) lbl.textContent = cb.checked ? "Yes" : "No";
        updateDependencies();
      });
    });

    // Dependency visibility
    updateDependencies();

    // Navigation
    var backBtn = document.getElementById("btn-back");
    var backLauncher = document.getElementById("btn-back-launcher");
    var nextBtn = document.getElementById("btn-next");

    if (backBtn) backBtn.addEventListener("click", function () {
      collectCurrentAnswers();
      state.currentStep--;
      render();
    });
    if (backLauncher) backLauncher.addEventListener("click", function () {
      state.phase = "launcher";
      render();
    });
    if (nextBtn) nextBtn.addEventListener("click", function () {
      if (!validateStep()) return;
      collectCurrentAnswers();
      if (state.currentStep < state.steps.length - 1) {
        state.currentStep++;
        render();
      } else {
        state.phase = "review";
        render();
      }
    });
  }

  function updateDependencies() {
    app.querySelectorAll("[data-depends-field]").forEach(function (group) {
      var field = group.getAttribute("data-depends-field");
      var value = group.getAttribute("data-depends-value");
      var el = document.getElementById("f-" + field);
      var current;
      if (el && el.type === "checkbox") {
        current = el.checked ? "true" : "false";
      } else if (el) {
        current = el.value;
      }
      group.style.display = current === value ? "" : "none";
    });
  }

  function collectCurrentAnswers() {
    var step = state.steps[state.currentStep];
    if (!step) return;
    step.fields.forEach(function (field) {
      var el = document.getElementById("f-" + field.id);
      if (!el) return;
      if (field.kind === "boolean") {
        state.answers[field.id] = el.checked;
      } else {
        state.answers[field.id] = el.value;
      }
    });
  }

  function validateStep() {
    var step = state.steps[state.currentStep];
    for (var i = 0; i < step.fields.length; i++) {
      var field = step.fields[i];
      if (!field.required) continue;
      // Skip hidden dependent fields
      var group = document.querySelector('[data-depends-field="' + field.depends_on?.field + '"]');
      if (field.depends_on && group && group.style.display === "none") continue;

      var el = document.getElementById("f-" + field.id);
      if (el && field.kind !== "boolean" && !el.value.trim()) {
        el.classList.add("error-field");
        el.focus();
        return false;
      }
    }
    return true;
  }

  // -- Review --

  function renderReview() {
    var progress = (state.steps.length + 1) + " / " + (state.steps.length + 1);
    var html = '<header><h1>Review</h1>';
    html += '<div class="progress">Step ' + progress + '</div></header>';
    html += '<main><div class="review-list">';

    state.steps.forEach(function (step) {
      html += '<div class="review-section"><h3>' + esc(step.title) + '</h3>';
      step.fields.forEach(function (field) {
        // Skip hidden
        if (field.depends_on) {
          var depVal = state.answers[field.depends_on.field];
          if (String(depVal) !== field.depends_on.value) return;
        }
        var val = state.answers[field.id];
        if (val === undefined || val === "") return;
        var display = field.kind === "boolean" ? (val ? "Yes" : "No") : String(val);
        html += '<div class="review-row"><span class="review-label">' + esc(field.label) + '</span><span class="review-value">' + esc(display) + '</span></div>';
      });
      html += '</div>';
    });

    html += '</div><div class="nav-buttons">';
    html += '<button class="btn btn-secondary" id="btn-back-review">Back</button>';
    html += '<button class="btn btn-primary" id="btn-execute">Execute</button>';
    html += '</div></main>';

    app.innerHTML = html;

    document.getElementById("btn-back-review").addEventListener("click", function () {
      state.currentStep = state.steps.length - 1;
      state.phase = "form";
      render();
    });
    document.getElementById("btn-execute").addEventListener("click", function () {
      executeWizard();
    });
  }

  // -- Execution --

  function executeWizard() {
    state.phase = "executing";
    render();

    fetch("/api/wizard/submit", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ answers: state.answers }),
    }).then(function () {
      return fetch("/api/wizard/execute", { method: "POST" });
    }).then(function (r) { return r.json(); })
      .then(function () {
        return fetch("/api/wizard/result");
      }).then(function (r) { return r.json(); })
      .then(function (result) {
        state.result = result;
        state.phase = "result";
        render();
      }).catch(function (err) {
        state.result = { success: false, stdout: "", stderr: err.message, exit_code: null };
        state.phase = "result";
        render();
      });
  }

  function renderExecuting() {
    app.innerHTML = '<div class="center-msg"><div class="spinner"></div><p>Executing wizard...</p></div>';
  }

  function renderResult() {
    var r = state.result;
    var statusClass = r && r.success ? "success" : "error";
    var statusText = r && r.success ? "Completed successfully" : "Failed";

    var html = '<header><h1>Result</h1></header><main>';
    html += '<div class="result-status ' + statusClass + '">' + statusText + '</div>';

    if (r && r.stdout) {
      html += '<div class="output-block"><h3>Output</h3><pre>' + esc(r.stdout) + '</pre></div>';
    }
    if (r && r.stderr) {
      html += '<div class="output-block"><h3>Log</h3><pre>' + esc(r.stderr) + '</pre></div>';
    }
    if (r && r.exit_code !== null && r.exit_code !== undefined) {
      html += '<p class="exit-code">Exit code: ' + r.exit_code + '</p>';
    }

    html += '<div class="nav-buttons">';
    html += '<button class="btn btn-secondary" id="btn-new">New Wizard</button>';
    html += '<button class="btn btn-secondary" id="btn-close">Close</button>';
    html += '</div></main>';

    app.innerHTML = html;

    document.getElementById("btn-new").addEventListener("click", function () {
      state.phase = "launcher";
      state.answers = {};
      state.currentStep = 0;
      render();
    });
    document.getElementById("btn-close").addEventListener("click", function () {
      fetch("/api/shutdown", { method: "POST" });
      app.innerHTML = '<div class="center-msg">Wizard closed.</div>';
    });
  }

  // -- Helpers --

  function esc(str) {
    var div = document.createElement("div");
    div.textContent = str || "";
    return div.innerHTML;
  }

  // -- Init --
  render();
})();
