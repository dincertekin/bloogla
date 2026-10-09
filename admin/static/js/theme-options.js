// The theme options page (admin/templates/theme_options.html): colors with a
// reset button, and images chosen from the media library.
//
// Saving swaps in a fresh copy of the form, so these listen on the whole
// page and find the field from the element that was used.
(() => {
    // Colors: show the code next to the swatch; "Reset" goes back to the default.
    document.addEventListener("input", (event) => {
        if (event.target.type !== "color") return;
        const code = event.target.parentElement.querySelector("code");
        if (code) code.textContent = event.target.value;
    });

    // Images: "Choose" opens the media library, "Remove" clears it.
    const setImage = (field, url) => {
        field.querySelector("input[type=hidden]").value = url;
        const preview = field.querySelector("[data-image-preview]");
        preview.src = url;
        preview.hidden = !url;
        field.querySelector("[data-image-remove]").hidden = !url;
    };

    document.addEventListener("click", (event) => {
        const reset = event.target.closest("[data-reset-color]");
        if (reset) {
            const input = document.getElementById(reset.dataset.resetColor);
            input.value = reset.dataset.default;
            input.dispatchEvent(new Event("input", { bubbles: true }));
        }

        const field = event.target.closest("[data-image-option]");
        if (!field) return;
        if (event.target.closest("[data-image-choose]")) openMediaPicker((url) => setImage(field, url));
        if (event.target.closest("[data-image-remove]")) setImage(field, "");
    });
})();
