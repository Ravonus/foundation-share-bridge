//! The `<html><head>…<body>…` shell wrapper shared by every HTML handler.

use chrono::Utc;

use crate::html::{assets::LOGO_MARK_SVG, styles::page::PAGE_STYLE};
use crate::util::text::escape_html;

pub fn render_page(title: &str, body_html: &str) -> String {
    let year = Utc::now().format("%Y").to_string();
    let mut out = String::with_capacity(8192 + body_html.len());
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n");
    out.push_str("  <meta charset=\"utf-8\" />\n");
    out.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n");
    out.push_str("  <title>");
    out.push_str(&escape_html(title));
    out.push_str(" · Agorix Share Bridge</title>\n");
    out.push_str("  <link rel=\"preconnect\" href=\"https://fonts.googleapis.com\" />\n");
    out.push_str("  <link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin />\n");
    out.push_str("  <link rel=\"stylesheet\" href=\"https://fonts.googleapis.com/css2?family=Fraunces:opsz,wght@9..144,400;9..144,500&family=Inter:wght@400;500;600&display=swap\" />\n");
    out.push_str("  <script type=\"module\" src=\"https://cdn.jsdelivr.net/npm/@google/model-viewer/dist/model-viewer.min.js\"></script>\n");
    out.push_str("  <style>:root{--font-inter:'Inter';--font-fraunces:'Fraunces';}");
    out.push_str(PAGE_STYLE);
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str("<div class=\"page-wrap\">\n");
    out.push_str("  <nav class=\"site-nav\"><div class=\"site-nav-inner\">");
    out.push_str("<a class=\"brand\" href=\"/\" aria-label=\"Agorix home\">");
    out.push_str(LOGO_MARK_SVG);
    out.push_str("<span class=\"brand-word\">Agorix</span>");
    out.push_str("<span class=\"brand-eyebrow\">share bridge</span>");
    out.push_str("</a>");
    out.push_str(
        "<div class=\"nav-links\">\
         <a href=\"/#status\">Status</a>\
         <a href=\"/#inventory\">Pins</a>\
         <a href=\"/#connection\">Connection</a>\
         <a href=\"/settings\">Settings</a>\
         </div>",
    );
    out.push_str("</div></nav>\n");
    out.push_str(body_html);
    out.push_str(
        "\n  <footer class=\"site-footer\"><div class=\"site-footer-inner\">\
        <div>\
          <div class=\"brand-row\">",
    );
    out.push_str(LOGO_MARK_SVG);
    out.push_str(
        "<span class=\"brand-word\">Agorix</span>\
          </div>\
          <p class=\"about\">Agorix is the broader preservation project. This local companion app keeps rescued Foundation roots pinned on your IPFS node and self-repairs anything that drops. Not affiliated with Foundation.</p>\
          <p class=\"tagline\">Local pin companion · Forever repair · Artist-aligned</p>\
        </div>\
        <div>\
          <p class=\"foot-col-label\">Bridge</p>\
          <ul class=\"foot-links\">\
            <li><a href=\"/#status\">Status</a></li>\
            <li><a href=\"/#inventory\">Local pins</a></li>\
            <li><a href=\"/#connection\">Connection</a></li>\
            <li><a href=\"/settings\">Settings</a></li>\
          </ul>\
        </div>\
      </div>\
      <div class=\"footer-meta\"><div class=\"footer-meta-inner\">\
        <p>© ",
    );
    out.push_str(&year);
    out.push_str(
        " Agorix</p>\
        <p>Independent · Decentralized · Artist-aligned</p>\
      </div></div>\
    </footer>\n",
    );
    out.push_str("</div>\n</body>\n</html>");
    out
}
