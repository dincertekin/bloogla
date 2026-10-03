// The settings page (admin/templates/settings.html): choosing the site icon.
(() => {
    const input = document.getElementById("site_icon");
    if (!input) return;
    const preview = document.getElementById("icon-preview");
    const empty = document.getElementById("icon-empty");
    const remove = document.getElementById("icon-remove");
    const setIcon = (url) => {
        input.value = url;
        preview.src = url;
        preview.hidden = !url;
        empty.hidden = !!url;
        remove.hidden = !url;
    };
    document.getElementById("icon-choose").addEventListener("click", () => openMediaPicker(setIcon));
    remove.addEventListener("click", () => setIcon(""));
})();
