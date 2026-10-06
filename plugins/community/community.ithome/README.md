# IT之家

This community cleanup plugin removes two standalone paragraphs from IT Home articles:

- The ticket-purchase cashback promotion mentioning “最会买”.
- The disclosure about external links in articles.

The rules match normalized paragraph text exactly, so similarly worded text elsewhere in the
article is left intact. The plugin matches `ithome.com` and its subdomains. Import this directory
from **Settings → Plugins**. Re-importing a newer copy with the same ID replaces the installed
copy.
