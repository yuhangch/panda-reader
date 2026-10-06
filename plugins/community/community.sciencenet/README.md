# ScienceNet

This community rule plugin handles two ScienceNet page artifacts:

- It removes the presentation-only header table that ScienceNet places before the article body.
  The table contains a duplicate headline and spacer rows; removing only the headline leaves a
  blank table in the reader. The selector is limited to the outer header table so article tables
  remain intact.
- It removes a `div` whose normalized text contains the distinctive opening phrase of ScienceNet's
  standalone 转载声明. This tolerates changes to the rest of the notice while remaining scoped to
  `div` elements; inline styles are stripped before cleanup, so the rule does not depend on style.

The plugin matches `sciencenet.cn` and its subdomains. Import this directory from
**Settings → Plugins**. Re-importing a newer copy with the same ID replaces the installed copy.
