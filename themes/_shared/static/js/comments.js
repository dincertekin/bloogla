// Comment form: note when the page was opened (the server uses it to spot
// bots), and send comments without reloading the page. Without JavaScript
// the form is sent the ordinary way and the page reloads.
//
// While sending, the button shows a spinner. When the comment is accepted, a
// check draws itself on the button before the thank-you appears; when it
// isn't (too long, or too many at once), the button shakes.
(() => {
    const openedAt = Date.now();
    const stamp = () =>
        document.querySelectorAll('.comment-form [name="ts"]').forEach((input) => (input.value = openedAt));
    stamp();

    const lessMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
    const wait = (ms) => new Promise((resolve) => setTimeout(resolve, lessMotion.matches ? 0 : ms));

    // A check mark drawn as a line, so CSS can draw it stroke by stroke.
    const drawnCheck = () => {
        const svgNs = "http://www.w3.org/2000/svg";
        const svg = document.createElementNS(svgNs, "svg");
        svg.setAttribute("viewBox", "0 0 24 24");
        svg.setAttribute("class", "button-check");
        svg.setAttribute("aria-hidden", "true");
        const path = document.createElementNS(svgNs, "path");
        path.setAttribute("d", "M5 12.5l4.5 4.5L19 7.5");
        svg.append(path);
        return svg;
    };

    const shake = (element) => {
        element.classList.remove("shake");
        void element.offsetWidth; // Start again even if it's already shaking.
        element.classList.add("shake");
    };

    document.addEventListener("submit", async (event) => {
        const form = event.target.closest(".comment-form");
        if (!form) return;
        event.preventDefault();
        const button = form.querySelector("button");
        // The label goes transparent; the spinner and check keep its color.
        button.style.setProperty("--ink", getComputedStyle(button).color);
        form.classList.add("is-sending");
        button.disabled = true;
        try {
            const response = await fetch(form.action, {
                method: "POST",
                body: new URLSearchParams(new FormData(form)),
            });
            // The server answers with the post page; take its comments section.
            const page = new DOMParser().parseFromString(await response.text(), "text/html");
            const fresh = page.getElementById("comments");
            if (!response.ok || !fresh) throw new Error("unexpected answer");

            // Not accepted (too long, or too many at once): keep what was written.
            const result = new URL(response.url).searchParams.get("comment");
            const refused = result === "invalid" || result === "slow";
            if (refused) {
                for (const name of ["name", "email", "content"]) {
                    fresh.querySelector(`[name="${name}"]`).value = form.querySelector(`[name="${name}"]`).value;
                }
            } else {
                form.classList.remove("is-sending");
                form.classList.add("is-done");
                button.append(drawnCheck());
                await wait(700);
            }
            document.getElementById("comments").replaceWith(fresh);
            stamp();
            fresh.querySelector(".comment-notice")?.scrollIntoView({ block: "center", behavior: "smooth" });
            if (refused) shake(fresh.querySelector(".comment-form button"));
        } catch {
            // Something unexpected: send it the ordinary way instead.
            form.submit();
        }
    });
})();
