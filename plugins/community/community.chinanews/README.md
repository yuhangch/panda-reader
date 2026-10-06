# 中国新闻网

This plugin removes the `.content_left_time` block from Chinanews article HTML. On the
current page template this block contains the duplicated publication date/source line,
font-size controls (`大字体` / `小字体`), and sharing controls. A second selector removes
font-size controls if the site places them outside that wrapper in a future template.

It does not remove article paragraphs or images. The rule runs only for hosts under
`chinanews.com.cn` and only in the cleanup stage, so it can clean article HTML supplied by
an RSS feed or extracted from a web page.

This community plugin removes the duplicated date/source row, font-size controls, and sharing
controls from China News Network article pages. It leaves article paragraphs and images intact.

Import this directory from **Settings → Plugins**. The application copies the plugin into its
user plugin directory. Disable or remove it there to reverse the change. Existing processed
article bodies are refreshed when opened after plugin settings change.
