// Comment form: note when the page was opened (the server uses it to spot
// bots), and send comments without reloading the page. Without JavaScript
// the form is sent the ordinary way and the page reloads.
(() => {
    const openedAt = Date.now();
    const stamp = () =>
        document.querySelectorAll('.comment-form [name="ts"]').forEach((input) => (input.value = openedAt));
    stamp();

    document.addEventListener("submit", async (event) => {
        const form = event.target.closest(".comment-form");
        if (!form) return;
        event.preventDefault();
        form.classList.add("is-sending");
        form.querySelector("button").disabled = true;
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
            if (result === "invalid" || result === "slow") {
                for (const name of ["name", "email", "content"]) {
                    fresh.querySelector(`[name="${name}"]`).value = form.querySelector(`[name="${name}"]`).value;
                }
            }
            document.getElementById("comments").replaceWith(fresh);
            stamp();
            fresh.querySelector(".comment-notice")?.scrollIntoView({ block: "center", behavior: "smooth" });
        } catch {
            // Something unexpected: send it the ordinary way instead.
            form.submit();
        }
    });
})();
