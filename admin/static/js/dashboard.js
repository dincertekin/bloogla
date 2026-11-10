// The Dashboard's Customize button: move the boxes (drag a box by its
// handle, or focus the handle and press the arrow keys) and untick the ones
// you don't need. Each change is saved for this person at once
// (POST /admin/dashboard/layout), so it looks the same on every device.

(() => {
    const openButton = document.querySelector("[data-dashboard-customize]");
    const panel = document.getElementById("dashboard-customize");
    const list = document.querySelector("[data-dashboard-sections]");
    if (!openButton || !panel || !list) return;

    const status = panel.querySelector(".dashboard-customize-status");
    const announcer = document.querySelector("[data-dashboard-announce]");
    const boxes = () => [...list.querySelectorAll("[data-section]")];
    const shownBoxes = () => boxes().filter((box) => !box.hidden);

    // ---- Opening and closing ----

    function setCustomizing(on) {
        panel.hidden = !on;
        openButton.setAttribute("aria-expanded", String(on));
        list.classList.toggle("is-customizing", on);
        list.querySelectorAll("[data-section-handle]").forEach((handle) => {
            handle.hidden = !on;
        });
        if (on) {
            panel.querySelector("input")?.focus();
        } else {
            openButton.focus();
        }
    }

    openButton.addEventListener("click", () => setCustomizing(panel.hidden));
    panel.querySelector("[data-dashboard-done]").addEventListener("click", () => setCustomizing(false));
    panel.addEventListener("keydown", (event) => {
        if (event.key === "Escape") setCustomizing(false);
    });

    // ---- Saving: the boxes in order, hidden ones with a "-" ----

    async function save() {
        const layout = boxes()
            .map((box) => (box.hidden ? "-" : "") + box.dataset.section)
            .join(",");
        status.replaceChildren();
        try {
            const response = await fetch("/admin/dashboard/layout", {
                method: "POST",
                headers: { "Content-Type": "application/x-www-form-urlencoded" },
                body: new URLSearchParams({ layout }),
            });
            if (!response.ok) throw new Error(String(response.status));
        } catch {
            const message = document.createElement("div");
            message.className = "alert alert-error";
            message.textContent = panel.dataset.textFailed;
            status.append(message);
        }
    }

    // ---- Showing and hiding boxes ----

    panel.querySelectorAll("[data-section-toggle]").forEach((checkbox) => {
        checkbox.addEventListener("change", () => {
            const box = list.querySelector(`[data-section="${checkbox.dataset.sectionToggle}"]`);
            if (!box) return;
            box.hidden = !checkbox.checked;
            save();
        });
    });

    // ---- Moving with the keyboard ----

    // Tells screen reader users where the box is now: "Most read: position 2 of 4".
    function announce(box) {
        const shown = shownBoxes();
        announcer.textContent = panel.dataset.textPosition
            .replace("{name}", box.dataset.name)
            .replace("{n}", String(shown.indexOf(box) + 1))
            .replace("{total}", String(shown.length));
    }

    // ---- Sliding: boxes glide to their new places instead of jumping ----

    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

    // Change the order with `reorder`, then let every box in `boxesToSlide`
    // glide from where it was to where it is now ("FLIP").
    function slide(boxesToSlide, reorder) {
        const before = new Map(boxesToSlide.map((box) => [box, box.getBoundingClientRect().top]));
        reorder();
        if (reducedMotion.matches) return;
        for (const [box, top] of before) {
            const distance = top - box.getBoundingClientRect().top;
            if (distance === 0) continue;
            box.animate(
                [{ transform: `translateY(${distance}px)` }, { transform: "translateY(0)" }],
                { duration: 200, easing: "cubic-bezier(0.2, 0, 0, 1)" },
            );
        }
    }

    // ---- Moving with the keyboard ----

    list.addEventListener("keydown", (event) => {
        const handle = event.target.closest("[data-section-handle]");
        if (!handle || !["ArrowUp", "ArrowDown"].includes(event.key)) return;
        event.preventDefault();
        const box = handle.closest("[data-section]");
        const shown = shownBoxes();
        const index = shown.indexOf(box);
        const up = event.key === "ArrowUp";
        if ((up && index === 0) || (!up && index === shown.length - 1)) return;
        slide(shown, () => {
            if (up) {
                list.insertBefore(box, shown[index - 1]);
            } else {
                shown[index + 1].after(box);
            }
        });
        // Moving an element can take the focus away from it.
        handle.focus();
        announce(box);
        save();
    });

    // ---- Moving with a mouse or a finger ----
    // The box follows the pointer; the others slide out of its way as it
    // passes their middle. Letting go settles it into its new place.

    /** Where the box would be without the drag's offset, from the page's top. */
    function restingTop(box) {
        const offset = box.style.transform;
        box.style.transform = "";
        const top = box.getBoundingClientRect().top + window.scrollY;
        box.style.transform = offset;
        return top;
    }

    list.addEventListener("pointerdown", (event) => {
        const handle = event.target.closest("[data-section-handle]");
        if (!handle || event.button !== 0) return;
        event.preventDefault();
        const box = handle.closest("[data-section]");
        const before = boxes().map((b) => b.dataset.section).join();
        const pointer = event.pointerId;
        // How far below the box's top the pointer took hold of it.
        const grip = event.pageY - restingTop(box);
        let lastY = event.clientY;
        box.classList.add("is-dragging");
        list.classList.add("is-sorting");

        function follow() {
            box.style.transform = `translateY(${lastY + window.scrollY - grip - restingTop(box)}px)`;
        }

        // Followed on the whole page, not on the handle: moving the box
        // takes it out of the page for an instant, and the handle would
        // miss the moment the button is released.
        const move = (moveEvent) => {
            if (moveEvent.pointerId !== pointer) return;
            lastY = moveEvent.clientY;
            // Near the top or bottom of the window, the page scrolls along.
            const edge = 60;
            if (lastY < edge) window.scrollBy(0, -12);
            if (lastY > window.innerHeight - edge) window.scrollBy(0, 12);

            // The box goes before the first other box whose middle is below the pointer.
            const others = shownBoxes().filter((other) => other !== box);
            const next = others.find((other) => {
                const rect = other.getBoundingClientRect();
                return lastY < rect.top + rect.height / 2;
            });
            const last = others[others.length - 1];
            const moves = next ? box.nextElementSibling !== next : last && last.nextElementSibling !== box;
            if (moves) {
                slide(others, () => (next ? list.insertBefore(box, next) : last.after(box)));
            }
            follow();
        };
        const keepUnderPointer = () => follow();

        const stop = (stopEvent) => {
            if (stopEvent.pointerId !== pointer) return;
            document.removeEventListener("pointermove", move);
            document.removeEventListener("pointerup", stop);
            document.removeEventListener("pointercancel", stop);
            window.removeEventListener("scroll", keepUnderPointer);
            list.classList.remove("is-sorting");

            // Settle: from under the pointer into its place.
            const offset = box.style.transform;
            box.style.transform = "";
            const settled = () => box.classList.remove("is-dragging");
            if (reducedMotion.matches || !offset) {
                settled();
            } else {
                box.animate([{ transform: offset }, { transform: "translateY(0)" }], {
                    duration: 180,
                    easing: "cubic-bezier(0.2, 0, 0, 1)",
                }).finished.then(settled, settled);
            }
            if (boxes().map((b) => b.dataset.section).join() !== before) {
                announce(box);
                save();
            }
        };
        document.addEventListener("pointermove", move);
        document.addEventListener("pointerup", stop);
        document.addEventListener("pointercancel", stop);
        window.addEventListener("scroll", keepUnderPointer, { passive: true });
    });
})();
