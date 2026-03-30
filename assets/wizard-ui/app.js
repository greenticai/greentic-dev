(function () {
  "use strict";

  var app = document.getElementById("app");
  var state = {
    phase: "launcher",
    wizardType: null,
    subAction: null,
    steps: [],
    currentStep: 0,
    answers: {},
    result: null,
  };

  // ── Render dispatcher ──

  function render() {
    switch (state.phase) {
      case "launcher": renderLauncher(); break;
      case "submenu": renderSubmenu(); break;
      case "form": renderFormStep(); break;
      case "review": renderReview(); break;
      case "executing": renderExecuting(); break;
      case "result": renderResult(); break;
    }
  }

  // ── Launcher ──

  function renderLauncher() {
    fetch("/api/launcher/options")
      .then(function (r) { return r.json(); })
      .then(function (data) {
        app.innerHTML =
          '<div class="fade-in">' +
            '<div class="brand">' +
              '<div class="brand-icon">' +
                '<svg width="32" height="32" viewBox="0 0 32 32" fill="none"><rect width="32" height="32" rx="8" fill="#25c39e"/><path d="M10 16.5L14 20.5L22 12.5" stroke="white" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/></svg>' +
              '</div>' +
              '<h1 class="brand-title">' + esc(data.title) + '</h1>' +
              '<p class="brand-desc">Select what you want to create or manage.</p>' +
            '</div>' +
            '<div class="card-group">' +
              renderOptionCard("pack", "Pack", "Build or update an application pack with flows and components.", data.options[0] ? data.options[0].label : "") +
              renderOptionCard("bundle", "Bundle", "Build or update a production bundle for deployment.", data.options[1] ? data.options[1].label : "") +
            '</div>' +
            renderDetectedProjects(data.detected || []) +
            '<div class="launcher-footer">' +
              '<button class="btn btn-ghost" data-action="exit">Close Wizard</button>' +
            '</div>' +
          '</div>';

        app.querySelectorAll("[data-action]").forEach(function (btn) {
          btn.addEventListener("click", function () {
            var action = btn.getAttribute("data-action");
            if (action === "exit") {
              fetch("/api/shutdown", { method: "POST" });
              app.innerHTML = '<div class="fade-in center-msg"><p>Wizard closed. You can close this tab.</p></div>';
            } else {
              selectWizard(action);
            }
          });
        });

        // Detected project click → go to update flow with path pre-filled
        app.querySelectorAll("[data-detected-kind]").forEach(function (btn) {
          btn.addEventListener("click", function () {
            var kind = btn.getAttribute("data-detected-kind");
            var path = btn.getAttribute("data-detected-path");
            openDetectedProject(kind, path);
          });
        });
      });
  }

  function renderOptionCard(value, title, desc) {
    return (
      '<button class="option-card" data-action="' + esc(value) + '">' +
        '<div class="option-card-icon">' + (value === "pack" ?
          '<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"/><path d="m3.3 7 8.7 5 8.7-5"/><path d="M12 22V12"/></svg>' :
          '<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m7.5 4.27 9 5.15"/><path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"/><path d="m3.3 7 8.7 5 8.7-5"/><path d="M12 22V12"/></svg>'
        ) + '</div>' +
        '<div class="option-card-content">' +
          '<span class="option-card-title">' + esc(title) + '</span>' +
          '<span class="option-card-desc">' + esc(desc) + '</span>' +
        '</div>' +
        '<div class="option-card-arrow">' +
          '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>' +
        '</div>' +
      '</button>'
    );
  }

  function openDetectedProject(kind, path) {
    var wizardType = kind; // "pack" or "bundle"
    var subAction = kind === "pack" ? "update_app" : "update";
    state.wizardType = wizardType;
    state.subAction = subAction;
    state.answers = {};
    state.currentStep = 0;

    // Pre-fill the path
    if (kind === "pack") {
      state.answers.pack_dir = path;
    } else {
      state.answers.bundle_target = path;
    }

    fetch("/api/launcher/select", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ selected_action: wizardType }),
    }).then(function () {
      return fetch("/api/wizard/submenu/select", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ selected_action: subAction }),
      });
    }).then(function () {
      return fetch("/api/wizard/steps");
    }).then(function (r) { return r.json(); })
      .then(function (data) {
        if (data && data.steps && data.steps.length > 0) {
          state.steps = data.steps;
          state.phase = "form";
          render();
        }
      });
  }

  function renderDetectedProjects(projects) {
    if (!projects || projects.length === 0) return "";
    var html =
      '<div class="detected-section">' +
        '<div class="detected-header">' +
          '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>' +
          '<span>Detected in current directory</span>' +
        '</div>' +
        '<div class="detected-list">';
    projects.forEach(function (p) {
      var icon = p.kind === "bundle" ?
        '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m7.5 4.27 9 5.15"/><path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"/><path d="m3.3 7 8.7 5 8.7-5"/><path d="M12 22V12"/></svg>' :
        '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"/><path d="m3.3 7 8.7 5 8.7-5"/><path d="M12 22V12"/></svg>';
      html +=
        '<button class="detected-item" data-detected-kind="' + esc(p.kind) + '" data-detected-path="' + esc(p.path) + '">' +
          '<span class="detected-icon">' + icon + '</span>' +
          '<span class="detected-info">' +
            '<span class="detected-name">' + esc(p.name) + '</span>' +
            '<span class="detected-path">' + esc(p.path) + '</span>' +
          '</span>' +
          '<span class="detected-badge">' + esc(p.kind) + '</span>' +
        '</button>';
    });
    html += '</div></div>';
    return html;
  }

  function selectWizard(action) {
    state.wizardType = action;
    state.subAction = null;
    state.answers = {};
    state.currentStep = 0;
    fetch("/api/launcher/select", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ selected_action: action }),
    }).then(function () {
      state.phase = "submenu";
      render();
    });
  }

  // ── Sub-menu ──

  function renderSubmenu() {
    fetch("/api/wizard/submenu")
      .then(function (r) { return r.json(); })
      .then(function (data) {
        if (!data) return;
        var html =
          '<div class="fade-in">' +
            '<div class="step-header">' +
              '<button class="btn btn-ghost btn-sm btn-back" id="btn-back-submenu">' +
                '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>' +
                ' Back' +
              '</button>' +
            '</div>' +
            '<div class="brand">' +
              '<h1 class="brand-title">' + esc(data.title) + '</h1>' +
              '<p class="brand-desc">Select an action to get started.</p>' +
            '</div>' +
            '<div class="card-group">';

        data.options.forEach(function (opt) {
          html +=
            '<button class="option-card" data-sub="' + esc(opt.value) + '">' +
              '<div class="option-card-content">' +
                '<span class="option-card-title">' + esc(opt.label) + '</span>' +
                '<span class="option-card-desc">' + esc(opt.description) + '</span>' +
              '</div>' +
              '<div class="option-card-arrow">' +
                '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>' +
              '</div>' +
            '</button>';
        });

        html += '</div></div>';
        app.innerHTML = html;

        document.getElementById("btn-back-submenu").addEventListener("click", function () {
          state.phase = "launcher";
          render();
        });

        app.querySelectorAll("[data-sub]").forEach(function (btn) {
          btn.addEventListener("click", function () {
            selectSubAction(btn.getAttribute("data-sub"));
          });
        });
      });
  }

  function selectSubAction(action) {
    state.subAction = action;
    state.answers = {};
    state.currentStep = 0;
    state.catalog = null;
    fetch("/api/wizard/submenu/select", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ selected_action: action }),
    }).then(function () {
      return fetch("/api/wizard/steps");
    }).then(function (r) { return r.json(); })
      .then(function (data) {
        if (data && data.steps && data.steps.length > 0) {
          state.steps = data.steps;
          // For extension flows, try loading default catalog to populate selects
          var isExt = action === "create_ext" || action === "update_ext" || action === "add_ext";
          if (isExt) {
            loadCatalogAndEnrichSteps(data.steps);
            return;
          }
          state.phase = "form";
          render();
        } else {
          app.innerHTML = '<div class="fade-in center-msg"><p>This wizard flow is not yet available in the web UI.</p><button class="btn btn-secondary" id="btn-back-unsupported">Back</button></div>';
          document.getElementById("btn-back-unsupported").addEventListener("click", function () {
            state.phase = "submenu";
            render();
          });
        }
      });
  }

  function loadCatalogAndEnrichSteps(steps) {
    // Find the catalog ref default value from steps
    var catalogRef = "file://docs/extensions_capability_packs.catalog.v1.json";
    steps.forEach(function (step) {
      step.fields.forEach(function (f) {
        if (f.id === "extension_catalog_ref" && f.default_value) {
          catalogRef = f.default_value;
        }
      });
    });

    fetch("/api/catalog/load", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ catalog_ref: catalogRef }),
    }).then(function (r) { return r.json(); })
      .then(function (catalog) {
        state.catalog = catalog;
        // Enrich type/template fields with choices from catalog
        steps.forEach(function (step) {
          step.fields.forEach(function (f) {
            if (f.id === "extension_type_id" && catalog.types && catalog.types.length > 0) {
              f.kind = "select";
              f.choices = catalog.types.map(function (t) {
                return { value: t.id, label: t.name + " — " + t.description };
              });
              f.default_value = catalog.types[0].id;
              f.placeholder = null;
            }
            if (f.id === "extension_template_id" && catalog.types && catalog.types.length > 0) {
              // Populate with all templates from first type; updated on type change
              var allTemplates = [];
              catalog.types.forEach(function (t) {
                t.templates.forEach(function (tmpl) {
                  allTemplates.push({ value: tmpl.id, label: t.id + " / " + tmpl.name });
                });
              });
              if (allTemplates.length > 0) {
                f.kind = "select";
                f.choices = allTemplates;
                f.default_value = allTemplates[0].value;
                f.placeholder = null;
              }
            }
          });
        });
        state.steps = steps;
        state.phase = "form";
        render();
      }).catch(function () {
        // Catalog load failed, fall back to text inputs
        state.steps = steps;
        state.phase = "form";
        render();
      });
  }

  // ── Multi-step Form ──

  function renderFormStep() {
    var step = state.steps[state.currentStep];
    if (!step) return;

    var totalSteps = state.steps.length + 1;
    var currentNum = state.currentStep + 1;
    var pct = Math.round((currentNum / totalSteps) * 100);

    var html =
      '<div class="fade-in">' +
        '<div class="step-header">' +
          '<button class="btn btn-ghost btn-sm btn-back" id="btn-back">' +
            '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>' +
            ' Back' +
          '</button>' +
          '<span class="step-badge">Step ' + currentNum + ' of ' + totalSteps + '</span>' +
        '</div>' +
        '<div class="progress-bar"><div class="progress-fill" style="width:' + pct + '%"></div></div>' +
        '<div class="card">' +
          '<div class="card-header">' +
            '<h2 class="card-title">' + esc(step.title) + '</h2>' +
            '<p class="card-desc">' + esc(step.description) + '</p>' +
          '</div>' +
          '<div class="card-content">' +
            '<form id="step-form" class="form-fields">';

    step.fields.forEach(function (field) {
      var depAttr = "";
      if (field.depends_on) {
        depAttr = ' data-depends-field="' + esc(field.depends_on.field) + '" data-depends-value="' + esc(field.depends_on.value) + '"';
      }
      html += '<div class="field"' + depAttr + '>';

      var val = state.answers[field.id] !== undefined ? state.answers[field.id] : (field.default_value || "");

      if (field.kind === "boolean") {
        var checked = val === "true" || val === true ? " checked" : "";
        html +=
          '<div class="field-row">' +
            '<label class="field-label" for="f-' + esc(field.id) + '">' + esc(field.label) + '</label>' +
            '<label class="switch">' +
              '<input type="checkbox" id="f-' + esc(field.id) + '" name="' + esc(field.id) + '"' + checked + ' />' +
              '<span class="switch-slider"></span>' +
            '</label>' +
          '</div>';
      } else {
        html += '<label class="field-label" for="f-' + esc(field.id) + '">' + esc(field.label);
        if (field.required) html += '<span class="required">*</span>';
        html += '</label>';

        if (field.kind === "select") {
          html += '<select id="f-' + esc(field.id) + '" name="' + esc(field.id) + '">';
          field.choices.forEach(function (c) {
            var sel = String(val) === c.value ? " selected" : "";
            html += '<option value="' + esc(c.value) + '"' + sel + '>' + esc(c.label) + '</option>';
          });
          html += '</select>';
        } else {
          html += '<input type="text" id="f-' + esc(field.id) + '" name="' + esc(field.id) + '" value="' + esc(String(val)) + '"';
          if (field.placeholder) html += ' placeholder="' + esc(field.placeholder) + '"';
          html += ' />';
        }
      }
      html += '</div>';
    });

    html +=
            '</form>' +
          '</div>' +
          '<div class="card-footer">' +
            '<button class="btn btn-primary" id="btn-next">Continue</button>' +
          '</div>' +
        '</div>' +
      '</div>';

    app.innerHTML = html;
    setupFormListeners();
  }

  function setupFormListeners() {
    app.querySelectorAll('input[type="checkbox"]').forEach(function (cb) {
      cb.addEventListener("change", function () { updateDependencies(); });
    });
    updateDependencies();

    document.getElementById("btn-back").addEventListener("click", function () {
      collectCurrentAnswers();
      if (state.currentStep > 0) {
        state.currentStep--;
        render();
      } else {
        state.phase = "submenu";
        render();
      }
    });

    document.getElementById("btn-next").addEventListener("click", function () {
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
      var current = el && el.type === "checkbox" ? (el.checked ? "true" : "false") : (el ? el.value : "");
      group.style.display = current === value ? "" : "none";
    });
  }

  function collectCurrentAnswers() {
    var step = state.steps[state.currentStep];
    if (!step) return;
    step.fields.forEach(function (field) {
      var el = document.getElementById("f-" + field.id);
      if (!el) return;
      state.answers[field.id] = field.kind === "boolean" ? el.checked : el.value;
    });
  }

  function clearErrors() {
    app.querySelectorAll(".field-error").forEach(function (e) { e.remove(); });
    app.querySelectorAll(".input-error").forEach(function (e) { e.classList.remove("input-error"); });
  }

  function showFieldError(el, msg) {
    el.classList.add("input-error");
    var err = document.createElement("p");
    err.className = "field-error";
    err.textContent = msg;
    el.parentElement.appendChild(err);
  }

  function validateStep() {
    clearErrors();
    var step = state.steps[state.currentStep];
    var firstErr = null;
    for (var i = 0; i < step.fields.length; i++) {
      var f = step.fields[i];
      if (f.kind === "boolean") continue;
      if (f.depends_on) {
        var g = app.querySelector('[data-depends-field="' + f.depends_on.field + '"]');
        if (g && g.style.display === "none") continue;
      }
      var el = document.getElementById("f-" + f.id);
      if (!el) continue;
      var val = el.value.trim();

      // Required check
      if (f.required && !val) {
        showFieldError(el, f.label + " is required.");
        if (!firstErr) firstErr = el;
        continue;
      }

      // Catalog ref scheme check
      if (f.id === "extension_catalog_ref" && val && !val.match(/^(file|https?|oci|fixture):\/\//)) {
        showFieldError(el, "Must start with file://, https://, http://, or oci://");
        if (!firstErr) firstErr = el;
      }
    }
    if (firstErr) { firstErr.focus(); return false; }
    return true;
  }

  // ── Review ──

  function renderReview() {
    var totalSteps = state.steps.length + 1;
    var html =
      '<div class="fade-in">' +
        '<div class="step-header">' +
          '<button class="btn btn-ghost btn-sm btn-back" id="btn-back-review">' +
            '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>' +
            ' Back' +
          '</button>' +
          '<span class="step-badge">Step ' + totalSteps + ' of ' + totalSteps + '</span>' +
        '</div>' +
        '<div class="progress-bar"><div class="progress-fill" style="width:100%"></div></div>' +
        '<div class="card">' +
          '<div class="card-header">' +
            '<h2 class="card-title">Review & Execute</h2>' +
            '<p class="card-desc">Review your configuration before executing.</p>' +
          '</div>' +
          '<div class="card-content">';

    state.steps.forEach(function (step) {
      html += '<div class="review-group"><h4 class="review-group-title">' + esc(step.title) + '</h4>';
      step.fields.forEach(function (field) {
        if (field.depends_on) {
          var depVal = state.answers[field.depends_on.field];
          if (String(depVal) !== field.depends_on.value) return;
        }
        var val = state.answers[field.id];
        if (val === undefined || val === "") return;
        var display = field.kind === "boolean" ? (val ? "Yes" : "No") : String(val);
        html +=
          '<div class="review-item">' +
            '<span class="review-key">' + esc(field.label) + '</span>' +
            '<span class="review-val">' + esc(display) + '</span>' +
          '</div>';
      });
      html += '</div>';
    });

    html +=
          '</div>' +
          '<div class="card-footer">' +
            '<button class="btn btn-primary btn-lg" id="btn-execute">' +
              '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12 7-7 7 7"/><path d="M12 19V5"/></svg>' +
              ' Execute' +
            '</button>' +
          '</div>' +
        '</div>' +
      '</div>';

    app.innerHTML = html;

    document.getElementById("btn-back-review").addEventListener("click", function () {
      state.currentStep = state.steps.length - 1;
      state.phase = "form";
      render();
    });
    document.getElementById("btn-execute").addEventListener("click", executeWizard);
  }

  // ── Execution ──

  function executeWizard() {
    state.phase = "executing";
    render();
    fetch("/api/wizard/submit", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ answers: state.answers }),
    }).then(function () {
      return fetch("/api/wizard/execute", { method: "POST" });
    }).then(function () {
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
    app.innerHTML =
      '<div class="fade-in center-msg">' +
        '<div class="spinner"></div>' +
        '<p class="executing-text">Running wizard...</p>' +
        '<p class="executing-sub">This may take a moment.</p>' +
      '</div>';
  }

  function renderResult() {
    var r = state.result;
    var ok = r && r.success;

    var html =
      '<div class="fade-in">' +
        '<div class="card">' +
          '<div class="card-header center">' +
            '<div class="result-icon ' + (ok ? "result-ok" : "result-err") + '">' +
              (ok ?
                '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><path d="m9 11 3 3L22 4"/></svg>' :
                '<svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/></svg>'
              ) +
            '</div>' +
            '<h2 class="card-title">' + (ok ? "Completed" : "Failed") + '</h2>' +
            '<p class="card-desc">' + (ok ? "Wizard executed successfully." : "Something went wrong during execution.") + '</p>' +
          '</div>' +
          '<div class="card-content">';

    if (r && r.stdout) {
      html += '<div class="output-section"><h4 class="output-title">Output</h4><pre class="output-pre">' + esc(r.stdout) + '</pre></div>';
    }
    if (r && r.stderr) {
      html += '<div class="output-section"><h4 class="output-title">Log</h4><pre class="output-pre stderr">' + esc(r.stderr) + '</pre></div>';
    }

    html +=
          '</div>' +
          '<div class="card-footer card-footer-split">' +
            '<button class="btn btn-secondary" id="btn-new">New Wizard</button>' +
            '<button class="btn btn-ghost" id="btn-close">Close</button>' +
          '</div>' +
        '</div>' +
      '</div>';

    app.innerHTML = html;

    document.getElementById("btn-new").addEventListener("click", function () {
      state.phase = "launcher";
      state.answers = {};
      state.currentStep = 0;
      render();
    });
    document.getElementById("btn-close").addEventListener("click", function () {
      fetch("/api/shutdown", { method: "POST" });
      app.innerHTML = '<div class="fade-in center-msg"><p>Wizard closed. You can close this tab.</p></div>';
    });
  }

  // ── Helpers ──

  function esc(str) {
    var d = document.createElement("div");
    d.textContent = str || "";
    return d.innerHTML;
  }

  render();
})();
