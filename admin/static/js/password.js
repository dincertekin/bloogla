// Live password checklist for forms that set a new password.
//
// Markup (see admin/templates/password_rules.html):
//   <input data-new-password>               the new password
//   <div class="password-check">            strength bar + checklist
//     <ul class="password-rules"><li data-rule="upper">...</li></ul>
//   <input data-confirm-password>           optional "repeat password" field
//   <span class="password-mismatch" hidden> shown when the two differ
//
// Each rule turns green as it's met. Submitting with something missing marks
// the missing rules red and stops the form. The server checks the same rules
// (PASSWORD_RULES in src/app/security.rs), so this is only for convenience.
(function () {
    const MIN_LENGTH = 12;

    const checks = {
        length: (p) => [...p].length >= MIN_LENGTH,
        lower: (p) => /\p{Ll}/u.test(p),
        upper: (p) => /\p{Lu}/u.test(p),
        number: (p) => /\p{N}/u.test(p),
        symbol: (p) => /[^\p{L}\p{N}\s]/u.test(p),
    };

    // Update the checklist and strength bar; returns true when every rule is met.
    function update(form) {
        const input = form.querySelector("[data-new-password]");
        const items = form.querySelectorAll(".password-rules [data-rule]");
        let met = 0;
        items.forEach((item) => {
            const check = checks[item.dataset.rule];
            const ok = check ? check(input.value) : true;
            item.classList.toggle("met", ok);
            if (ok) met++;
        });

        const box = form.querySelector(".password-check");
        if (box) {
            box.querySelectorAll(".password-meter span").forEach((bar, i) => {
                bar.classList.toggle("on", input.value !== "" && i < met);
            });
            const strength =
                input.value === "" ? "" : met === items.length ? "strong" : met >= 3 ? "fair" : "weak";
            box.dataset.strength = strength;
        }
        return met === items.length;
    }

    function passwordsMatch(form) {
        const input = form.querySelector("[data-new-password]");
        const confirm = form.querySelector("[data-confirm-password]");
        return !confirm || confirm.value === input.value;
    }

    document.addEventListener("input", (event) => {
        const form = event.target.form;
        if (!form || !form.querySelector("[data-new-password]")) return;
        update(form);
        const mismatch = form.querySelector(".password-mismatch");
        // Once shown, hide the mismatch message as soon as it's fixed.
        if (mismatch && !mismatch.hidden && passwordsMatch(form)) mismatch.hidden = true;
    });

    // Capture phase, so this runs before htmx sends the form.
    document.addEventListener(
        "submit",
        (event) => {
            const form = event.target;
            const input = form.querySelector("[data-new-password]");
            if (!input) return;

            const rulesMet = update(form);
            const matches = passwordsMatch(form);
            form.querySelector(".password-rules")?.classList.toggle("checked", !rulesMet);
            const mismatch = form.querySelector(".password-mismatch");
            if (mismatch) mismatch.hidden = matches;

            if (!rulesMet || !matches) {
                event.preventDefault();
                event.stopPropagation();
                (rulesMet ? form.querySelector("[data-confirm-password]") : input).focus();
            }
        },
        true,
    );

    // Forms reset after a successful save start fresh.
    document.addEventListener("reset", (event) => {
        const form = event.target;
        if (!form.querySelector || !form.querySelector("[data-new-password]")) return;
        setTimeout(() => {
            update(form);
            form.querySelector(".password-rules")?.classList.remove("checked");
        });
    });
})();
