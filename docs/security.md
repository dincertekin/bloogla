# Security

How Bloogla protects a site, and what to keep in mind when running one. To
report a vulnerability, see [SECURITY.md](../SECURITY.md).

- **Content Security Policy:** The admin panel, login and setup only run scripts from Bloogla's own files (no inline scripts, no `eval`), so injected HTML can't run code. Admin JavaScript lives in `admin/static/js/`; a test fails if inline scripts return to the templates.
- **CSRF:** Cross-site `POST`/`DELETE` requests are rejected using the browser's `Sec-Fetch-Site` header, falling back to an `Origin` check against `BLOOGLA_BASE_URL`.
- **Rate Limiting:** Per-address limits on login and comments. Sign-in, two-factor code checks and current-password checks in Profile share an account limit, including concurrent requests. After 5 failed attempts in 15 minutes, an account accepts one attempt per minute from anywhere, which stops guessing spread across many addresses (accounts are slowed, never locked, so nobody can lock the owner out). Behind a reverse proxy on the same machine, make it overwrite `X-Forwarded-For` so visitors can't fake their address.
- **Visibility:** Drafts and not-yet-due scheduled posts are hidden from every public page, search, RSS and sitemap. Publish times are entered and shown in the site's time zone (**Settings**).
- **Backups:** Admins can download everything (database and images) as one .zip in **Settings → Backup**; to restore, stop Bloogla, unpack it into Bloogla's folder and start again. On the server there are also automatic daily copies of the database in `data/backups/`, and `bloogla backup` makes one any time.
- **Forgotten passwords:** **Forgot password?** on the sign-in page emails a link (when email is set up). It works once, for an hour, and only its hash is stored; the page answers the same whether or not the address has an account. Using it signs the account out everywhere; two-factor login stays on. Any password or email change cancels older reset links, including requests already in progress. Without email, the page explains who can help, and `bloogla reset-password` works on the server.
- **Uploads:** Files are checked by content (JPEG, PNG, GIF, WebP only), stored under random names, and photos are resized, stripped of metadata, and get an 800px-wide copy for phones.
- **Themes:** Only files in a theme's `static/` folder are public; templates and `theme.toml` are not served.
- **Two-factor login (optional, recommended):** Turn it on in Profile with your current password and a code from any authenticator app; you get 10 one-time recovery codes and other devices are signed out. Making new recovery codes asks for your current password and also signs out other devices. Nobody is forced to use it. Admins can turn it off for someone who lost their phone, and `bloogla disable-2fa EMAIL` works on the server.
- **Email changes:** Changing your sign-in and recovery address in Profile asks for your current password, cancels older reset links and signs out other devices. Changing just your name or language doesn't ask for a password.
- **Local files:** On Linux and macOS, databases and backups are readable only by the account running Bloogla (`600`); its `data/` and `data/backups/` folders are private (`700`). Startup also tightens existing files in those locations.
- **Stored secrets:** The mail server password and two-factor keys are encrypted with a key in `data/secret.key`. Keep that file with your backups; without it you re-enter the mail password and set up two-factor login again.
- **Setup:** A fresh site can only be set up with the one-time code from the setup link Bloogla prints at start.
- **Passwords:** At least 12 characters with a lowercase letter, an uppercase letter, a number and a symbol. Forms show what's missing as you type, and the server checks again. Rules live in `src/app/security.rs`.
- **Sessions:** HTTP-only, same-site cookies (HTTPS-only in production); expired sessions are removed hourly. Changing a password (or an admin resetting it) signs that account out on every other device.
- **Security log:** Sign-ins, failed sign-ins, password and role changes and new people are logged as `security event=...` lines. Find them with `grep "security event"` in the log (or `docker logs bloogla`).
- **Dependencies:** CI runs `cargo audit` on every push and weekly, and Dependabot proposes updates.
