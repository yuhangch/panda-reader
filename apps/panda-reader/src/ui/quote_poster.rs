#[cfg(not(test))]
use crate::ui::window::ReaderWindow;
use base64::Engine as _;
#[cfg(not(test))]
use gpui_kit::base::StyledExt as _;
#[cfg(not(test))]
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    v_flex,
};
#[cfg(not(test))]
use gpui_kit::*;

const POSTER_WIDTH: u32 = 1080;
static POSTER_FONT_DB: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> =
    std::sync::OnceLock::new();

#[derive(Clone)]
#[cfg(not(test))]
pub(in crate::ui) struct QuotePoster {
    pub image: Image,
    pub png: Vec<u8>,
}

#[cfg(not(test))]
impl QuotePoster {
    pub fn generate(
        quote: String,
        title: String,
        source: String,
        url: Option<String>,
        context_html: String,
        feed_icon: Option<Vec<u8>>,
    ) -> Result<Self, String> {
        let (before, after) = context_around_selection(&context_html, &quote);
        let svg = poster_svg(
            &quote,
            &title,
            &source,
            url.as_deref(),
            &before,
            &after,
            feed_icon.as_deref(),
        )?;
        let png = render_svg_png(&svg)?;
        let image = Image::from_bytes(ImageFormat::Png, png.clone());
        Ok(Self { image, png })
    }
}

fn render_svg_png(svg: &str) -> Result<Vec<u8>, String> {
    let mut options = resvg::usvg::Options::default();
    options.font_family = "Noto Sans SC".to_owned();
    options.fontdb = POSTER_FONT_DB
        .get_or_init(|| {
            let mut fonts = resvg::usvg::fontdb::Database::new();
            fonts.load_font_data(include_bytes!("../../assets/fonts/NotoSansSC-VF.ttf").to_vec());
            std::sync::Arc::new(fonts)
        })
        .clone();
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &options)
        .map_err(|error| format!("Could not lay out poster: {error}"))?;
    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| "Could not allocate poster image".to_owned())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|error| format!("Could not encode poster: {error}"))
}

fn poster_svg(
    quote: &str,
    title: &str,
    source: &str,
    url: Option<&str>,
    before: &str,
    after: &str,
    feed_icon: Option<&[u8]>,
) -> Result<String, String> {
    let before_lines = fit_context_lines(wrap_text(before, 20.0), true);
    let quote_lines = wrap_text(quote, 16.0);
    let after_lines = fit_context_lines(wrap_text(after, 20.0), false);
    let before_start = 330_u32;
    let before_height = before_lines.len() as u32 * 62;
    let quote_start = before_start + before_height + if before_lines.is_empty() { 80 } else { 30 };
    let quote_line_height = 82_u32;
    let after_start = quote_start + quote_lines.len() as u32 * quote_line_height + 30;
    let after_height = after_lines.len() as u32 * 62;
    let body_end = after_start + after_height;
    let footer_top = (body_end + 150).max(1160);
    let height = footer_top + 280;
    let font = "Noto Sans SC";
    let app_icon_data = base64::engine::general_purpose::STANDARD
        .encode(include_bytes!("../../assets/app-icon.png"));
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{POSTER_WIDTH}" height="{height}" viewBox="0 0 {POSTER_WIDTH} {height}">
<rect width="100%" height="100%" fill="#fff"/>
<image x="120" y="118" width="42" height="42" preserveAspectRatio="xMidYMid meet" href="data:image/png;base64,{app_icon_data}"/>
<text x="178" y="151" fill="#77736f" stroke="#77736f" stroke-width="0.45" paint-order="stroke fill" font-size="23" font-weight="700" letter-spacing="3" font-family="{font}">PANDA READER · 引文</text>
"##,
    );
    for (index, line) in before_lines.iter().enumerate() {
        svg.push_str(&format!(
            "<text x=\"120\" y=\"{}\" fill=\"#c6c6c6\" stroke=\"#c6c6c6\" stroke-width=\"0.6\" paint-order=\"stroke fill\" font-size=\"42\" font-weight=\"500\" font-family=\"{font}\">{}</text>\n",
            before_start + index as u32 * 62,
            xml_escape(line),
        ));
    }
    for (index, line) in quote_lines.iter().enumerate() {
        svg.push_str(&format!(
            "<text x=\"120\" y=\"{}\" fill=\"#111111\" stroke=\"#111111\" stroke-width=\"1.2\" paint-order=\"stroke fill\" font-size=\"52\" font-weight=\"700\" font-family=\"{font}\">{}</text>\n",
            quote_start + index as u32 * quote_line_height,
            xml_escape(line),
        ));
    }
    for (index, line) in after_lines.iter().enumerate() {
        svg.push_str(&format!(
            "<text x=\"120\" y=\"{}\" fill=\"#c6c6c6\" stroke=\"#c6c6c6\" stroke-width=\"0.6\" paint-order=\"stroke fill\" font-size=\"42\" font-weight=\"500\" font-family=\"{font}\">{}</text>\n",
            after_start + index as u32 * 62,
            xml_escape(line),
        ));
    }

    let feed_lines = wrap_text(source, 24.0);
    let title_lines = wrap_text(title, 28.0);
    let qr = url
        .filter(|url| !url.trim().is_empty())
        .map(|url| qrcodegen::QrCode::encode_text(url, qrcodegen::QrCodeEcc::Medium))
        .transpose()
        .map_err(|_| "Article link is too long for a QR code".to_owned())?;
    svg.push_str(&format!(
        r##"<line x1="96" y1="{footer_top}" x2="984" y2="{footer_top}" stroke="#eeeeee" stroke-width="2"/>
"##,
    ));
    let text_x = if let Some(icon) = feed_icon {
        let icon_data = base64::engine::general_purpose::STANDARD.encode(icon);
        svg.push_str(&format!(
            r##"<defs><clipPath id="feed-icon"><circle cx="156" cy="{}" r="36"/></clipPath></defs>
<image x="120" y="{}" width="72" height="72" preserveAspectRatio="xMidYMid slice" href="data:image/png;base64,{icon_data}" clip-path="url(#feed-icon)"/>
"##,
            footer_top + 106,
            footer_top + 70,
        ));
        220
    } else {
        120
    };
    svg.push_str(&format!(
        r##"<text x="{text_x}" y="{}" fill="#242424" stroke="#242424" stroke-width="0.55" paint-order="stroke fill" font-size="29" font-weight="700" font-family="{font}">{}</text>
<text x="{text_x}" y="{}" fill="#4c4945" stroke="#4c4945" stroke-width="0.45" paint-order="stroke fill" font-size="23" font-weight="600" font-family="{font}">{}</text>
<text x="{text_x}" y="{}" fill="#77736f" stroke="#77736f" stroke-width="0.4" paint-order="stroke fill" font-size="23" font-weight="600" font-family="{font}">{}</text>
"##,
        footer_top + 89,
        xml_escape(feed_lines.first().map(String::as_str).unwrap_or("")),
        footer_top + 128,
        xml_escape(title_lines.first().map(String::as_str).unwrap_or("")),
        footer_top + 158,
        xml_escape(title_lines.get(1).map(String::as_str).unwrap_or("")),
    ));
    if let Some(qr) = qr {
        let qr_size = 128.0_f32;
        let quiet_zone = 12.0_f32;
        let module_count = qr.size() as f32;
        let module_size = qr_size / module_count;
        let qr_x = 862.0_f32;
        let qr_y = footer_top as f32 + 36.0;
        svg.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"#fff\" stroke=\"#eeeeee\" stroke-width=\"2\"/>\n<path fill=\"#171717\" d=\"",
            qr_x - quiet_zone,
            qr_y - quiet_zone,
            qr_size + quiet_zone * 2.0,
            qr_size + quiet_zone * 2.0,
        ));
        for y in 0..qr.size() {
            for x in 0..qr.size() {
                if qr.get_module(x, y) {
                    let x = qr_x + x as f32 * module_size;
                    let y = qr_y + y as f32 * module_size;
                    svg.push_str(&format!(
                        "M{x:.2} {y:.2}h{module_size:.2}v{module_size:.2}h-{module_size:.2}z"
                    ));
                }
            }
        }
        svg.push_str("\"/>\n");
    }
    svg.push_str("</svg>");
    Ok(svg)
}

fn context_around_selection(html: &str, selected_text: &str) -> (String, String) {
    if html.trim().is_empty() || selected_text.trim().is_empty() {
        return (String::new(), String::new());
    }
    let fragment = scraper::Html::parse_fragment(html);
    let content = fragment.root_element().text().collect::<Vec<_>>().join(" ");
    let content = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let selection = selected_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if selection.is_empty() {
        return (String::new(), String::new());
    }
    let Some(start) = content.find(&selection) else {
        return (String::new(), String::new());
    };
    let end = start + selection.len();
    let before = content[..start]
        .chars()
        .rev()
        .take(76)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    let after = content[end..].chars().take(76).collect::<String>();
    (
        if before.trim().is_empty() {
            String::new()
        } else {
            before.trim().to_owned()
        },
        if after.trim().is_empty() {
            String::new()
        } else {
            after.trim().to_owned()
        },
    )
}

fn fit_context_lines(mut lines: Vec<String>, keep_end: bool) -> Vec<String> {
    if lines.len() <= 2 {
        return lines;
    }
    if keep_end {
        lines.drain(..lines.len() - 2);
    } else {
        lines.truncate(2);
    }
    lines
}

fn wrap_text(text: &str, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut width = 0.0;
    for character in text
        .split_whitespace()
        .flat_map(|word| word.chars().chain([' ']))
    {
        let char_width = if character.is_ascii() { 0.52 } else { 1.0 };
        if width + char_width > max_width && !current.is_empty() {
            lines.push(current.trim_end().to_owned());
            current.clear();
            width = 0.0;
        }
        current.push(character);
        width += char_width;
    }
    let last = current.trim_end();
    if !last.is_empty() {
        lines.push(last.to_owned());
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(not(test))]
impl ReaderWindow {
    pub(in crate::ui) fn begin_quote_poster(
        &mut self,
        quote: String,
        title: String,
        source: String,
        url: Option<String>,
        context_html: String,
        feed_icon: Option<Vec<u8>>,
        cx: &mut Context<Self>,
    ) {
        match QuotePoster::generate(quote, title, source, url, context_html, feed_icon) {
            Ok(poster) => self.quote_poster = Some(poster),
            Err(error) => self.set_error(error),
        }
        cx.notify();
    }

    pub(in crate::ui) fn render_quote_poster(
        &self,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let Some(poster) = &self.quote_poster else {
            return div().into_any_element();
        };
        let scrim = cx.theme().background.opacity(0.62);
        let preview_image = poster.image.clone();
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(scrim)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.quote_poster = None;
                    cx.notify();
                }),
            )
            .child(
                v_flex()
                    .w(px(520.))
                    .max_h(px(760.))
                    .gap_3()
                    .p_5()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_lg()
                                    .font_semibold()
                                    .child(self.t("Quote poster")),
                            )
                            .child(
                                Button::new("quote-poster-close")
                                    .small()
                                    .ghost()
                                    .label(self.t("Close"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.quote_poster = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .max_h(px(570.))
                            .overflow_y_scrollbar()
                            .child(
                                img(std::sync::Arc::new(preview_image))
                                    .w(px(360.))
                                    .rounded(px(6.)),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .pt_1()
                            .child(
                                Button::new("quote-poster-copy")
                                    .small()
                                    .secondary()
                                    .label(self.t("Copy image"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(poster) = &this.quote_poster {
                                            cx.write_to_clipboard(ClipboardItem::new_image(
                                                &poster.image,
                                            ));
                                            this.set_flash(this.t("Poster copied"), cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new("quote-poster-save")
                                    .small()
                                    .primary()
                                    .label(self.t("Save PNG"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let Some(poster) = this.quote_poster.clone() else {
                                            return;
                                        };
                                        let weak = cx.entity().downgrade();
                                        cx.spawn(async move |_, cx| {
                                            if let Some(file) = rfd::AsyncFileDialog::new()
                                                .set_file_name("panda-reader-quote.png")
                                                .add_filter("PNG image", &["png"])
                                                .save_file()
                                                .await
                                            {
                                                match file.write(&poster.png).await {
                                                    Ok(()) => {
                                                        let _ = weak.update(cx, |this, cx| {
                                                            this.set_flash(
                                                                this.t("Poster saved"),
                                                                cx,
                                                            )
                                                        });
                                                    }
                                                    Err(error) => {
                                                        let _ = weak.update(cx, |this, _| {
                                                            this.set_error(error.to_string())
                                                        });
                                                    }
                                                }
                                            }
                                        })
                                        .detach();
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{poster_svg, wrap_text};

    #[test]
    fn poster_escapes_article_text_and_embeds_qr_for_url() {
        let svg = poster_svg(
            "<hello & goodbye>",
            "A title",
            "A source",
            Some("https://example.com/a"),
            "faded context before",
            "faded context after",
            None,
        )
        .expect("poster svg should be generated");
        assert!(svg.contains("&lt;hello &amp; goodbye&gt;"));
        assert!(svg.contains("fill=\"#c6c6c6\""));
        assert!(svg.contains("<path fill=\"#171717\""));
        assert!(svg.contains("width=\"152\""));
        assert!(svg.contains("font-family=\"Noto Sans SC\""));
        assert!(svg.contains("font-weight=\"600\""));
        let logo_at = svg
            .find("<image x=\"120\" y=\"118\"")
            .expect("Panda Reader logo should be in the header");
        let brand_at = svg.find("PANDA READER · 引文").unwrap();
        assert!(logo_at < brand_at);
        assert!(!svg.contains("id=\"feed-icon\""));
        assert!(svg.contains("<text x=\"120\""));
    }

    #[test]
    fn poster_wraps_cjk_and_keeps_short_quotes_on_one_line() {
        assert_eq!(wrap_text("A short quote", 25.0), ["A short quote"]);
        assert!(wrap_text(&"字".repeat(60), 25.0).len() > 1);
    }

    #[test]
    fn poster_renders_as_a_png_image() {
        let svg = poster_svg(
            "中国铁路西安局集团有限公司联合安康市文旅局推出高铁文旅优惠活动",
            "新线开通，坐高铁享景区优惠！",
            "中国铁路",
            Some("https://example.com/article"),
            "又有浸润岁月的人文古迹，一起来看看吧。",
            "即日起至2026年12月31日，安康市所辖景区均可享受优惠。",
            Some(include_bytes!("../../assets/app-icon.png")),
        )
        .expect("poster svg should be generated");
        let icon_at = svg.find("id=\"feed-icon\"").unwrap();
        let source_at = svg.rfind("中国铁路").unwrap();
        assert!(icon_at < source_at);
        assert!(svg.contains("<text x=\"220\""));
        let png = super::render_svg_png(&svg).expect("poster should render to png");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn selected_quote_gets_nearby_article_context() {
        let (before, after) = super::context_around_selection(
            "<p>前面的灰色上下文</p><p>突出显示的内容</p><p>后面的灰色上下文</p>",
            "突出显示的内容",
        );
        assert!(before.contains("前面的灰色上下文"));
        assert!(after.contains("后面的灰色上下文"));
    }
}
