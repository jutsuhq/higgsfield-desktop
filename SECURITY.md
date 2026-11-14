# Security

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub: open the repository's **Security** tab and choose **Report a
vulnerability** (private vulnerability reporting). The report stays visible only to the maintainers until an advisory
is published. Don't open a public issue or pull request for a vulnerability.

Include the app version, your operating system, steps to reproduce and what an attacker gains. You'll get an answer in
the advisory thread; fixes ship in a new release, and the advisory is published once that release is out.

Only the latest release is supported.

## Threat model

Jutsu Studio is a desktop client for Higgsfield on Windows and Linux. It has no server and no account of its own: you
bring your own Higgsfield account, either by signing in (OAuth with PKCE in your browser, through Higgsfield's MCP
server) or with a Higgsfield API key. Generations are billed to that account.

**Secrets.** API keys, access and refresh tokens and the OAuth client registration are stored only in the system's
credential store: Windows Credential Manager, or the Secret Service (GNOME Keyring, KWallet) on Linux. They are never
written to the settings, the database, files or logs, and they never reach the app's web view. Logs redact
credentials and signed URLs.

**Network.** The app contacts Higgsfield, GitHub and Hugging Face for downloads you agree to, and the host of a link
you paste:

| Host                                                                 | What is sent                                                                                                                                                                                                                          |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `api.higgsfield.ai`                                                  | With an API key: job requests, uploads of the source video, reference images and prompt, status polling                                                                                                                               |
| `mcp.higgsfield.ai` and the OAuth authorization server it advertises | With a sign-in: the OAuth flow (in your browser and the token endpoint), then the same jobs as MCP tool calls                                                                                                                         |
| Higgsfield storage (presigned URLs returned by the two above)        | Upload of inputs, download of results and preview media                                                                                                                                                                               |
| `github.com` and its release downloads                               | Updates: once a day (unless turned off in Settings) `latest.json` of this repository's latest release, and the update you install, checked against the release key. Upscale on this computer: the Real-ESRGAN release, once you agree |
| `huggingface.co`                                                     | Subtitles on this computer: the Whisper Small model, once you agree                                                                                                                                                                   |
| The host of a link you paste or drop                                 | A plain download of that file, as an input                                                                                                                                                                                            |

There is no telemetry, analytics or crash reporting, and no other host is contacted. Links in the app open in your
browser only when you click them. During sign-in the app listens on `127.0.0.1` for the OAuth redirect and stops
listening when the sign-in ends. The web view gets media from a server on `127.0.0.1` that serves only the app's own
folders.

**Agents.** Agents reach the app only through the `jutsu` helper over a named pipe (Windows) or a Unix socket in the
user's runtime folder (Linux), and only while Settings › Agents › Allow agents is on. They never see the credentials,
can't delete anything, and every paid request is checked against the daily agent budget and the person's own cap.

**Local data.** Everything else stays on your computer:

- Results: `Videos/Jutsu Studio`, or a folder you choose.
- History, prepared inputs, settings and downloaded engines: the app's data folder (`%APPDATA%\com.jutsustudio.desktop`
  on Windows, `~/.local/share/com.jutsustudio.desktop` on Linux).
- Short-lived scratch files: the system's temporary folder, and hidden `.<job id>.partial` files next to a result while
  it downloads.

**Out of scope.** Higgsfield's own services and how they handle uploaded media; a compromised computer or user account
(anything that can read your credential store or your files can read the app's).
