// The theme options page (admin/templates/theme_options.html): colors with a
// reset button, and images chosen from the media library.
(() => {
    // Colors: show the code next to the swatch; "Reset" goes back to the default.
    document.querySelectorAll("input[type=color]").forEach((input) => {
        const code = input.parentElement.querySelector("code");
        input.addEventListener("input", () => (code.textContent = input.value));
    });
    document.querySelectorAll("[data-reset-color]").forEach((button) => {
        button.addEventListener("click", () => {
            const input = document.getElementById(button.dataset.resetColor);
            input.value = button.dataset.default;
            input.dispatchEvent(new Event("input"));
        });
    });

    // Images: "Choose" opens the media library, "Remove" clears it.
    document.querySelectorAll("[data-image-option]").forEach((field) => {
        const input = field.querySelector("input[type=hidden]");
        const preview = field.querySelector("[data-image-preview]");
        const remove = field.querySelector("[data-image-remove]");
        const setImage = (url) => {
            input.value = url;
            preview.src = url;
            preview.hidden = !url;
            remove.hidden = !url;
        };
        field.querySelector("[data-image-choose]").addEventListener("click", () => openMediaPicker(setImage));
        remove.addEventListener("click", () => setImage(""));
    });
})();
