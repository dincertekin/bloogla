// Comment form: note when the page was opened (the server uses it to spot bots).
document.querySelectorAll('.comment-form [name="ts"]').forEach((input) => (input.value = Date.now()));
