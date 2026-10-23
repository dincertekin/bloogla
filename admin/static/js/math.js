// Draws the formulas in a post ($…$ in a sentence, $$…$$ on their own
// lines) with KaTeX. Bloogla adds this script only to pages that have one.
// A formula KaTeX can't read stays as typed, shown in red.
for (const formula of document.querySelectorAll("[data-math-style]")) {
    katex.render(formula.textContent, formula, {
        displayMode: formula.dataset.mathStyle === "display",
        throwOnError: false,
    });
}
