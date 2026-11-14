use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use url::Url;

use crate::mcp::http::query_items;

/// The page the browser tab shows once the authorization server redirects to the app's loopback listener: the
/// outcome, the app's icon, and a button back to the app.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallbackPage {
    /// Opens the app from the browser, through its URL scheme; `None` leaves the button out.
    pub app_url: Option<Url>,
    /// The app icon as PNG; `None` shows the signature gradient instead.
    pub icon_png: Option<Vec<u8>>,
}

impl CallbackPage {
    pub fn html(&self, callback: &Url) -> String {
        let failed = query_items(callback).iter().any(|(name, _)| name == "error");
        // The app still checks `state` and exchanges the code, so success here is only the approval.
        let title = if failed {
            "Sign-in was not completed"
        } else {
            "Access approved"
        };
        let detail = if failed {
            "Return to Jutsu Studio to try again, or connect with an API key instead."
        } else {
            "Jutsu Studio is finishing the sign-in."
        };
        let icon = self.icon_png.as_ref().map_or_else(
            || r#"<div class="mark"></div>"#.to_owned(),
            |png| {
                format!(
                    r#"<img src="data:image/png;base64,{}" alt="">"#,
                    STANDARD.encode(png)
                )
            },
        );
        // A check or a cross, drawn so the page needs nothing from the network.
        let badge = if failed {
            r#"<span class="badge fail"><svg viewBox="0 0 16 16"><path d="M5 5l6 6M11 5l-6 6"/></svg></span>"#
        } else {
            r#"<span class="badge"><svg viewBox="0 0 16 16"><path d="M3.5 8.5l3 3 6-7"/></svg></span>"#
        };
        let button = self.app_url.as_ref().map_or_else(String::new, |url| {
            let label = if failed { "Return to" } else { "Open" };
            format!(
                r#"<a class="open" href="{}">{label} Jutsu Studio</a>"#,
                escaped(url.as_str())
            )
        });
        format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Jutsu Studio</title>\n\
             <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
             <meta name=\"color-scheme\" content=\"dark\">\n\
             <style>{STYLE}</style></head><body><main>\n\
             <div class=\"icon\">{icon}{badge}</div>\n\
             <h1>{title}</h1><p>{detail}</p>{button}\n\
             <p class=\"hint\">You can close this tab.</p>\n\
             </main></body></html>"
        )
    }
}

const STYLE: &str = "*{box-sizing:border-box}
body{margin:0;min-height:100vh;display:grid;place-items:center;padding:24px;color:#fff;
background:#060607 radial-gradient(70% 55% at 50% -10%,rgba(139,92,246,.22),transparent 70%) no-repeat;
font:15px/1.5 -apple-system,BlinkMacSystemFont,\"SF Pro Text\",system-ui,sans-serif;
-webkit-font-smoothing:antialiased}
main{width:100%;max-width:400px;padding:44px 32px 32px;text-align:center;border-radius:24px;
background:rgba(255,255,255,.035);border:1px solid rgba(255,255,255,.08);
box-shadow:0 30px 90px rgba(0,0,0,.55)}
.icon{position:relative;width:96px;height:96px;margin:0 auto 26px}
.icon img{width:96px;height:96px;display:block}
.mark{width:96px;height:96px;border-radius:22px;
background:linear-gradient(135deg,#8b5cf6,#d946ef 55%,#f472b6)}
.badge{position:absolute;right:-4px;bottom:-4px;width:32px;height:32px;border-radius:50%;
display:grid;place-items:center;background:#d4ff2a;border:3px solid #0d0d0f}
.badge svg{width:16px;height:16px;fill:none;stroke:#0b0b0c;stroke-width:2.2;
stroke-linecap:round;stroke-linejoin:round}
.badge.fail{background:#ff5f57}.badge.fail svg{stroke:#fff}
h1{margin:0 0 8px;font-size:24px;font-weight:650;letter-spacing:-.015em}
p{margin:0;color:rgba(255,255,255,.6)}
.open{display:block;margin-top:30px;padding:13px 20px;border-radius:12px;background:#d4ff2a;
color:#0b0b0c;font-weight:600;text-decoration:none;transition:filter .15s}
.open:hover{filter:brightness(1.07)}.open:active{filter:brightness(.95)}
.hint{margin-top:16px;font-size:13px;color:rgba(255,255,255,.38)}";

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_redirect_shows_a_failure_page() {
        let failed = Url::parse("http://127.0.0.1:1/callback?error=access_denied&state=s").unwrap();
        let succeeded = Url::parse("http://127.0.0.1:1/callback?code=c&state=s").unwrap();
        assert!(CallbackPage::default().html(&failed).contains("not completed"));
        assert!(
            CallbackPage::default()
                .html(&succeeded)
                .contains("Access approved")
        );
    }

    #[test]
    fn the_page_links_back_to_the_app_and_shows_its_icon() {
        let succeeded = Url::parse("http://127.0.0.1:1/callback?code=c&state=s").unwrap();
        let page = CallbackPage {
            app_url: Some(Url::parse("jutsustudio://").unwrap()),
            icon_png: Some(vec![1, 2, 3]),
        };
        let html = page.html(&succeeded);
        assert!(
            html.contains(r#"<a class="open" href="jutsustudio://">Open Jutsu Studio</a>"#),
            "{html}"
        );
        assert!(html.contains("data:image/png;base64,AQID"));
        assert!(
            !CallbackPage::default()
                .html(&succeeded)
                .contains(r#"class="open""#)
        );
    }
}
