// Code blocks in posts: syntax colours (Prism, in prism.js) and a Copy button.
// Only loaded on pages that contain code. Button text comes from the data-
// attributes on this script's tag, so it's translated.
(() => {
    const text = document.currentScript?.dataset ?? {};
    const label = text.copy || "Copy";

    document.querySelectorAll(".post-body pre").forEach((pre) => {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "copy-code";
        button.textContent = label;
        button.addEventListener("click", async () => {
            try {
                await navigator.clipboard.writeText(pre.querySelector("code")?.innerText ?? pre.innerText);
                button.textContent = text.copied || label;
            } catch {
                button.textContent = text.copyFailed || label;
            }
            setTimeout(() => (button.textContent = label), 1500);
        });
        pre.append(button);
    });

    if (window.Prism) Prism.highlightAll();
})();
