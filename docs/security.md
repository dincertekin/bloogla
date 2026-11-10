# Security

How Bloogla protects your site. To report a problem, see
[SECURITY.md](../SECURITY.md).

## Signing in

- **Passwords** need 12 characters with a lowercase letter, an uppercase
  letter, a number and a symbol.
- **Guessing is slowed down.** After 5 wrong passwords in 15 minutes, an
  account accepts one try per minute. Accounts are never locked, so nobody
  can lock you out.
- **Two-factor login** is optional and recommended (Profile). You get 10
  recovery codes. An admin, or `bloogla disable-2fa EMAIL`, can turn it off
  for someone who lost their phone.
- **Forgot password?** emails a link that works once, for an hour. Using it
  signs that account out everywhere.
- **Changing a password or email** asks for the current password and signs
  the account out on other devices.
- **Setup** needs the one-time code from the link Bloogla prints when it
  starts.
- **Sign-ins and changes to accounts are logged.** Find them with
  `grep "security event"` in the log (or `docker logs bloogla`).

## Uploads and imports

- **Images** are checked by their content (JPEG, PNG, GIF, WebP; up to 20 MB
  and 12,000 pixels). Photos are turned upright, made at most 2,000 pixels
  wide, and lose their metadata: where they were taken, the camera, the
  date.
- **Videos** are checked by their content (MP4 or WebM, up to 100 MB). An
  MP4 loses where it was filmed and the phone's name; the video itself isn't
  changed.
- **Every file gets a random name.** Names and paths inside uploaded files
  are never trusted.
- **WordPress imports** are read as data, never run. Images are only copied
  from public internet addresses (never this server or a private network),
  with limits on size, time and redirects. A backup is saved first.
- **Restoring a backup** only takes the database and the uploads from the
  `.zip`, and saves your site as it is now first.

## Your data

- **Drafts and scheduled posts** stay hidden everywhere: pages, search, RSS
  and the sitemap. A draft you share has a private link only.
- **The mail password, the storage secret key and two-factor keys** are
  encrypted with `data/secret.key`. Keep that file with your backups.
- **Subscribers** confirm their address with a link before they get
  anything; addresses nobody confirmed are deleted after 7 days. Only admins
  see the list. Every email has a one-click unsubscribe link.
- **Copying backups to storage** only sends them to an `https://` address,
  signed with your keys, and never follows a redirect elsewhere.
- **On Linux, `data/`** can only be opened by the account running Bloogla.
- **Analytics** use no cookies and store no IP addresses.

## Behind the scenes

- **Strict Content Security Policy:** the admin runs only Bloogla's own
  scripts, so text someone slips into a page can't run code.
- **Forged requests are refused:** changes must come from your own site
  (checked with the browser's `Sec-Fetch-Site` and `Origin` headers).
- **Code for every page** (Settings → Site) accepts only `<meta>` tags and a
  few safe `<link>` tags. Scripts and styles are refused.
- **Themes** only publish their `static/` folder.
- **Updates are signed.** Bloogla installs only programs signed with
  Bloogla's release key.
- **Dependencies** are checked for known problems on every change and every
  week (`cargo audit`, Dependabot).

**Behind a reverse proxy:** make it overwrite `X-Forwarded-For`, so visitors
can't fake their address. Behind a CDN like Cloudflare, all visitors share
one limit unless the proxy passes on their real address.
