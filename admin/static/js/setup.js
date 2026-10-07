// The setup page (admin/templates/setup.html), one step at a time:
// 1. site type, 2. details, 3. account. "Next" checks the step's fields
// first. Without JavaScript the page shows every step at once.
(() => {
    const form = document.querySelector("[data-setup-steps]");
    if (!form) return;
    const steps = [...form.querySelectorAll("[data-step]")];
    const markers = [...document.querySelectorAll("[data-step-marker]")];
    let current = 1;

    function show(number, { focus = true } = {}) {
        current = number;
        steps.forEach((step) => (step.hidden = Number(step.dataset.step) !== number));
        markers.forEach((marker) => {
            const n = Number(marker.dataset.stepMarker);
            marker.classList.toggle("done", n < number);
            marker.classList.toggle("current", n === number);
            if (n === number) marker.setAttribute("aria-current", "step");
            else marker.removeAttribute("aria-current");
        });
        if (focus) {
            const first = steps[number - 1].querySelector(
                "input:not([type=hidden]):not([type=radio]), input[type=radio]:checked, select"
            );
            if (first) first.focus();
        }
        window.scrollTo({ top: 0 });
    }

    // The step's fields are filled in correctly (shows the browser's message if not).
    function stepIsValid(number) {
        const invalid = [...steps[number - 1].querySelectorAll("input, select")].find(
            (field) => !field.checkValidity()
        );
        if (invalid) invalid.reportValidity();
        return !invalid;
    }

    form.addEventListener("click", (event) => {
        if (event.target.closest("[data-next]") && stepIsValid(current)) show(current + 1);
        if (event.target.closest("[data-back]")) show(current - 1);
    });

    // Enter goes to the next step; only the last step creates the site.
    form.addEventListener("keydown", (event) => {
        if (event.key !== "Enter" || current === steps.length) return;
        if (event.target.matches("input:not([type=radio])")) {
            event.preventDefault();
            if (stepIsValid(current)) show(current + 1);
        }
    });

    document.querySelector(".setup-progress").hidden = false;
    form.querySelectorAll(".setup-buttons, [data-back]").forEach((el) => (el.hidden = false));
    // After a refused try, start on the step with the problem.
    show(Number(form.dataset.startStep) || 1, { focus: false });
})();
