use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    ZhCn,
    ZhTw,
    Japanese,
    French,
    German,
}

impl Language {
    pub const ALL: [Self; 6] = [
        Self::English,
        Self::ZhCn,
        Self::ZhTw,
        Self::Japanese,
        Self::French,
        Self::German,
    ];

    pub fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::ZhCn => "简体中文",
            Self::ZhTw => "繁體中文",
            Self::Japanese => "日本語",
            Self::French => "Français",
            Self::German => "Deutsch",
        }
    }

    pub fn translator_code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::ZhCn => "zh-Hans",
            Self::ZhTw => "zh-Hant",
            Self::Japanese => "ja",
            Self::French => "fr",
            Self::German => "de",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|language| *language == self)
            .unwrap_or(0)
    }
}

impl SearchableListItem for Language {
    type Value = Self;

    fn title(&self) -> SharedString {
        SharedString::from(self.native_name())
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

pub fn text(language: Language, english: &'static str) -> &'static str {
    match language {
        Language::English => english,
        Language::ZhCn => lookup(ZH_CN, english).unwrap_or(english),
        Language::ZhTw => lookup(ZH_TW, english).unwrap_or(english),
        Language::Japanese => lookup(JA, english).unwrap_or(english),
        Language::French => lookup(FR, english).unwrap_or(english),
        Language::German => lookup(DE, english).unwrap_or(english),
    }
}

pub fn format(language: Language, english: &'static str, arg: impl std::fmt::Display) -> String {
    text(language, english).replacen("{}", &arg.to_string(), 1)
}

pub fn error(language: Language, message: String) -> String {
    let table = match language {
        Language::English => return message,
        Language::ZhCn => ERROR_ZH_CN,
        Language::ZhTw => ERROR_ZH_TW,
        Language::Japanese => ERROR_JA,
        Language::French => ERROR_FR,
        Language::German => ERROR_DE,
    };
    let mut translated = message;
    for (english, localized) in table {
        translated = translated.replace(english, localized);
    }
    translated
}

fn lookup(table: &'static [(&'static str, &'static str)], english: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(key, _)| *key == english)
        .map(|(_, value)| *value)
}

const ZH_CN: &[(&str, &str)] = &[
    ("Add feed", "添加订阅"),
    ("Expand sidebar", "展开侧栏"),
    ("Collapse sidebar", "收起侧栏"),
    ("Settings", "设置"),
    ("All Articles", "全部文章"),
    ("Unread", "未读"),
    ("Starred", "收藏"),
    ("Star", "收藏"),
    ("Read Later", "稍后读"),
    ("Feeds", "订阅源"),
    ("Refresh feeds", "刷新订阅"),
    ("Feed", "订阅"),
    ("Loading articles…", "正在载入文章…"),
    ("No articles yet", "这里还没有文章"),
    (
        "Connect Miniflux in Settings to see your articles here",
        "在设置中连接 Miniflux 后，文章会出现在这里",
    ),
    ("Remove star", "取消收藏"),
    ("Save for later", "稍后读"),
    ("Remove from Read Later", "移出稍后读"),
    ("Reading", "阅读"),
    ("Currently reading", "阅读中"),
    ("Extract full text", "提取全文"),
    ("Re-extract full text", "重新提取全文"),
    ("Hide images", "隐藏图片"),
    ("Show images", "显示图片"),
    ("Open original", "打开原文"),
    ("Translate", "翻译"),
    ("Show original", "查看原文"),
    ("Show translation", "查看译文"),
    ("Translating…", "正在翻译…"),
    ("Translation ready", "译文已就绪"),
    ("Translation", "翻译"),
    ("Display mode", "显示模式"),
    ("Immersive", "沉浸式对照"),
    ("Translation only", "仅译文"),
    ("Bilingual", "双语"),
    ("Translated", "译文"),
    ("Switch", "切换"),
    (
        "Immersive shows original and translation together, like Immersive Translate.",
        "沉浸式对照会在原文下方显示译文，类似沉浸式翻译。",
    ),
    ("Toggle immersive translation", "切换沉浸式翻译"),
    ("Provider", "提供商"),
    ("Azure API key", "Azure API 密钥"),
    ("Azure region", "Azure 区域"),
    ("Volcengine", "火山引擎"),
    ("Azure Translator", "Azure 翻译"),
    ("Access Key ID", "Access Key ID"),
    ("Secret Access Key", "Secret Access Key"),
    (
        "Use Access Key ID / Secret from Volcengine IAM. Service: translate.",
        "在火山引擎访问控制中创建密钥。机器翻译服务名：translate。",
    ),
    ("Save translation settings", "保存翻译设置"),
    ("Translation settings saved", "翻译设置已保存"),
    (
        "Configure a translation provider in Settings before translating.",
        "请先在设置中配置翻译提供商，再进行翻译。",
    ),
    ("Coming soon", "即将支持"),
    (
        "Choose a translation provider for article text.",
        "选择用于翻译文章正文的提供商。",
    ),
    (
        "Interface language and article translation language can differ.",
        "界面语言与文章译文语言可以分开设置。",
    ),
    ("Article language", "文章语言"),
    (
        "Translate article bodies into this language.",
        "将文章正文翻译成此语言。",
    ),
    ("Start reading", "开始阅读"),
    (
        "Select an article from the list to begin",
        "从左侧选择一篇文章，静下来读一会儿",
    ),
    ("Back to Reader", "返回阅读"),
    ("SETTINGS", "设置"),
    ("General", "常规"),
    ("Reset settings", "重置设置"),
    ("Restore defaults", "恢复默认"),
    ("Reset app settings now?", "现在重置应用设置吗？"),
    ("Settings restored to defaults.", "设置已恢复为默认值。"),
    (
        "Restore interface and translation settings, including saved translation keys, to defaults. Feeds, articles and Provider connections are kept.",
        "将界面和翻译设置（包括已保存的翻译密钥）恢复为默认值。订阅、文章和 Provider 连接会保留。",
    ),
    ("Appearance", "外观"),
    ("About", "关于"),
    ("Development build", "开发版"),
    ("Remove", "移除"),
    (
        "Remove this feed and its local articles? It will also be removed from Miniflux.",
        "删除此订阅及本地文章？Miniflux 订阅也会从服务器删除。",
    ),
    ("Confirm removal", "确认移除"),
    ("Cancel", "取消"),
    (
        "Connect Miniflux to manage feeds and reading status on the server. Articles remain available offline.",
        "连接 Miniflux 后，订阅和阅读状态由服务器管理，文章保存在本机供离线阅读。",
    ),
    ("Server URL", "服务器地址"),
    ("Update connection", "更新连接"),
    ("Connect and sync", "连接并同步"),
    ("Disconnect", "断开连接"),
    ("Feed management", "订阅管理"),
    ("Sync now", "立即同步"),
    ("Import OPML", "导入 OPML"),
    ("Export OPML", "导出 OPML"),
    ("Added feeds", "已添加的订阅"),
    ("Search added feeds", "搜索已添加的订阅"),
    ("No feeds found", "没有找到订阅"),
    ("Theme", "主题"),
    (
        "Choose a color theme for reading.",
        "选择阅读环境的颜色主题。",
    ),
    ("Dark theme", "深色主题"),
    ("Light theme", "浅色主题"),
    ("Change theme  ›", "更换主题  ›"),
    ("Branding", "品牌标识"),
    ("Show panda icon", "显示熊猫图标"),
    (
        "Display the panda in the top-left corner.",
        "在左上角显示熊猫图标。",
    ),
    ("Interface size", "界面尺寸"),
    (
        "Adjust text and controls in the sidebar, list and settings.",
        "调整侧栏、列表和设置页的文字及控件大小。",
    ),
    ("Layout", "布局"),
    ("Collapse feed sidebar", "收起订阅侧栏"),
    ("Leave more room for articles.", "为文章留出更多横向空间。"),
    ("Collapsed", "已收起"),
    ("Expanded", "已展开"),
    ("Full text", "全文"),
    ("Auto extract full text", "自动提取全文"),
    (
        "Fetch the original page when opening an article without full content.",
        "打开尚无全文的文章时，自动抓取原文页面。",
    ),
    ("Content extractor", "正文抽取引擎"),
    (
        "Library used when extracting full text. Re-extract to compare.",
        "提取全文时使用的库。切换后请重新提取以对比效果。",
    ),
    ("Choose folder icon", "选择文件夹图标"),
    ("Book", "书本"),
    ("Globe", "地球"),
    ("Bookmark", "书签"),
    ("Move folder up", "文件夹上移"),
    ("Move folder down", "文件夹下移"),
    ("Readability (dom_smoothie)", "Readability (dom_smoothie)"),
    ("Defuddle (decruft)", "Defuddle (decruft)"),
    ("Trafilatura", "Trafilatura"),
    ("Heuristic", "启发式选择器"),
    ("On", "开"),
    ("Off", "关"),
    ("Interface text", "界面文字"),
    (
        "Article and interface text follow the interface size.",
        "阅读区与应用控件跟随界面尺寸。",
    ),
    ("Smaller", "缩小"),
    ("Larger", "放大"),
    (
        "Feeds, articles and settings are stored on this device.",
        "订阅、文章与设置均保存在本机。",
    ),
    ("Papr reference", "Papr 参考"),
    (
        "Papr informed our feed parsing, article extraction, and store design. Panda Reader's implementation is independent and does not include papr-core.",
        "Panda Reader 在订阅源解析、正文提取和 store 设计上参考了 Papr；代码为独立实现，不包含 papr-core。",
    ),
    ("TTY7 reference", "TTY7 参考"),
    (
        "TTY7 inspired our packaging and theme design.",
        "Panda Reader 的打包方式和主题设计参考了 TTY7。",
    ),
    ("Choose a reading palette", "选择喜欢的阅读配色"),
    ("Language", "语言"),
    (
        "Choose the language used throughout the app.",
        "选择应用界面的语言。",
    ),
    ("Slate Workspace", "灰蓝 · 工作台"),
    ("Feed URL https://…", "订阅地址 https://…"),
    ("Search articles", "搜索文章"),
    (
        "Connecting and syncing Miniflux…",
        "正在连接并同步 Miniflux…",
    ),
    (
        "Disconnected from Miniflux; cached articles remain available",
        "已断开 Miniflux；本地缓存仍可阅读",
    ),
    ("Adding feed…", "正在添加订阅…"),
    ("Feed added", "订阅已添加"),
    ("Removing feed…", "正在删除订阅…"),
    (
        "Feed removed from Miniflux and locally",
        "已从 Miniflux 和本地删除订阅",
    ),
    ("Syncing Miniflux…", "正在同步 Miniflux…"),
    ("Refreshing local feeds…", "正在刷新本地订阅…"),
    ("Extracting full text…", "正在提取全文…"),
    ("Full text extracted", "已提取全文"),
    ("OPML exported", "OPML 已导出"),
    ("Could not save settings: {}", "保存设置失败：{}"),
    ("Connected to Miniflux: {}", "已连接 Miniflux：{}"),
    ("Synced {} articles", "已同步 {} 篇文章"),
    ("Imported {} feeds", "已导入 {} 个订阅"),
    ("Connected: {}", "已连接：{}"),
    (
        "{} feeds. Additions and removals sync to Miniflux when connected.",
        "{} 个订阅源。连接 Miniflux 时，添加和移除会同步到服务器。",
    ),
    ("{} articles", "{} 篇文章"),
    ("{} unread", "{} 篇未读"),
    ("Show articles", "查看文章"),
    ("Copy feed title", "复制订阅名称"),
    ("Copy feed URL", "复制订阅地址"),
    ("Open feed URL", "打开订阅地址"),
    ("Remove feed", "移除订阅"),
    ("Open", "打开"),
    ("Mark as read", "标为已读"),
    ("Mark as unread", "标为未读"),
    ("Copy title", "复制标题"),
    ("Copy link", "复制链接"),
    ("Mark all as read", "全部标为已读"),
    ("Read all", "全部已读"),
    ("Load more articles", "加载更多文章"),
    ("Edit feed", "编辑订阅"),
    ("Edit feed…", "编辑订阅…"),
    ("Title", "标题"),
    ("Folder", "文件夹"),
    ("Feed URL", "订阅地址"),
    ("Save", "保存"),
    ("Feed updated", "订阅已更新"),
    ("Feed refreshed", "订阅已刷新"),
    ("Refreshing feed…", "正在刷新订阅…"),
    ("Marking all as read…", "正在全部标为已读…"),
    ("Marked {} articles as read", "已将 {} 篇文章标为已读"),
    ("Link copied", "链接已复制"),
    ("Title copied", "标题已复制"),
    ("Vim navigation", "Vim 导航"),
    (
        "Use j/k, s, u, l and other single keys when not typing.",
        "未在输入框中时，可用 j/k、s、u、l 等单键导航。",
    ),
    ("Keyboard", "键盘"),
    (
        "Open the Command Palette with Ctrl/Cmd+K anytime.",
        "随时可用 Ctrl/Cmd+K 打开命令面板。",
    ),
    ("Type a command…", "输入命令…"),
    ("No matching commands", "没有匹配的命令"),
    (
        "↑↓ navigate · Enter run · Esc close",
        "↑↓ 选择 · Enter 执行 · Esc 关闭",
    ),
    ("Next article", "下一篇"),
    ("Previous article", "上一篇"),
    ("Next unread", "下一未读"),
    ("Previous unread", "上一未读"),
    ("Go to All Articles", "转到全部文章"),
    ("Go to Unread", "转到未读"),
    ("Go to Starred", "转到收藏"),
    ("Go to Read Later", "转到稍后读"),
    ("Toggle read", "切换已读"),
    ("Toggle star", "切换收藏"),
    ("Toggle Read Later", "切换稍后读"),
    ("Toggle hide images", "切换隐藏图片"),
    ("Copy link", "复制链接"),
    ("Copy title", "复制标题"),
    ("Command Palette", "命令面板"),
    ("Keyboard shortcuts", "键盘快捷键"),
    ("Open Settings", "打开设置"),
    ("Open Appearance", "打开外观"),
    ("Open Reading settings", "打开阅读设置"),
    ("Open About", "打开关于"),
    ("Focus search", "聚焦搜索"),
    ("Toggle sidebar", "切换侧栏"),
    ("Navigate", "导航"),
    ("Article", "文章"),
    ("View", "视图"),
    ("Application", "应用"),
];

const ZH_TW: &[(&str, &str)] = &[
    ("Choose folder icon", "選擇資料夾圖示"),
    ("Folder", "資料夾"),
    ("Book", "書本"),
    ("Globe", "地球"),
    ("Bookmark", "書籤"),
    ("Add feed", "新增訂閱"),
    ("Expand sidebar", "展開側欄"),
    ("Collapse sidebar", "收合側欄"),
    ("Settings", "設定"),
    ("All Articles", "全部文章"),
    ("Unread", "未讀"),
    ("Starred", "收藏"),
    ("Star", "收藏"),
    ("Read Later", "稍後讀"),
    ("Feeds", "訂閱來源"),
    ("Refresh feeds", "重新整理訂閱"),
    ("Feed", "訂閱"),
    ("Loading articles…", "正在載入文章…"),
    ("No articles yet", "這裡還沒有文章"),
    (
        "Connect Miniflux in Settings to see your articles here",
        "在設定中連接 Miniflux 後，文章會出現在這裡",
    ),
    ("Remove star", "取消收藏"),
    ("Save for later", "稍後讀"),
    ("Remove from Read Later", "移出稍後讀"),
    ("Reading", "閱讀"),
    ("Currently reading", "閱讀中"),
    ("Extract full text", "擷取全文"),
    ("Re-extract full text", "重新擷取全文"),
    ("Hide images", "隱藏圖片"),
    ("Show images", "顯示圖片"),
    ("Open original", "開啟原文"),
    ("Translate", "翻譯"),
    ("Show original", "查看原文"),
    ("Show translation", "查看譯文"),
    ("Translating…", "正在翻譯…"),
    ("Translation ready", "譯文已就緒"),
    ("Translation", "翻譯"),
    ("Provider", "提供商"),
    ("Azure API key", "Azure API 金鑰"),
    ("Azure region", "Azure 區域"),
    ("Save translation settings", "儲存翻譯設定"),
    ("Translation settings saved", "翻譯設定已儲存"),
    (
        "Configure a translation provider in Settings before translating.",
        "請先在設定中配置翻譯提供商，再進行翻譯。",
    ),
    ("Coming soon", "即將支援"),
    (
        "Choose a translation provider for article text.",
        "選擇用於翻譯文章正文的提供商。",
    ),
    (
        "Interface language and article translation language can differ.",
        "介面語言與文章譯文語言可以分開設定。",
    ),
    ("Article language", "文章語言"),
    (
        "Translate article bodies into this language.",
        "將文章正文翻譯成此語言。",
    ),
    ("Start reading", "開始閱讀"),
    (
        "Select an article from the list to begin",
        "從左側選擇一篇文章，靜下來讀一會兒",
    ),
    ("Back to Reader", "返回閱讀"),
    ("SETTINGS", "設定"),
    ("General", "一般"),
    ("Reset settings", "重設設定"),
    ("Restore defaults", "恢復預設值"),
    ("Reset app settings now?", "現在重設應用程式設定嗎？"),
    ("Settings restored to defaults.", "設定已恢復為預設值。"),
    (
        "Restore interface and translation settings, including saved translation keys, to defaults. Feeds, articles and Provider connections are kept.",
        "將介面與翻譯設定（包含已儲存的翻譯金鑰）恢復為預設值。訂閱、文章與 Provider 連線會保留。",
    ),
    ("Appearance", "外觀"),
    ("About", "關於"),
    ("Development build", "開發版"),
    ("Remove", "移除"),
    (
        "Remove this feed and its local articles? It will also be removed from Miniflux.",
        "刪除此訂閱及本機文章？Miniflux 訂閱也會從伺服器刪除。",
    ),
    ("Confirm removal", "確認移除"),
    ("Cancel", "取消"),
    (
        "Connect Miniflux to manage feeds and reading status on the server. Articles remain available offline.",
        "連接 Miniflux 後，訂閱與閱讀狀態由伺服器管理，文章保存在本機供離線閱讀。",
    ),
    ("Server URL", "伺服器網址"),
    ("Update connection", "更新連線"),
    ("Connect and sync", "連接並同步"),
    ("Disconnect", "中斷連線"),
    ("Feed management", "訂閱管理"),
    ("Sync now", "立即同步"),
    ("Import OPML", "匯入 OPML"),
    ("Export OPML", "匯出 OPML"),
    ("Added feeds", "已新增的訂閱"),
    ("Search added feeds", "搜尋已新增的訂閱"),
    ("No feeds found", "找不到訂閱"),
    ("Theme", "主題"),
    (
        "Choose a color theme for reading.",
        "選擇閱讀環境的色彩主題。",
    ),
    ("Dark theme", "深色主題"),
    ("Light theme", "淺色主題"),
    ("Change theme  ›", "更換主題  ›"),
    ("Branding", "品牌標識"),
    ("Show panda icon", "顯示熊貓圖示"),
    (
        "Display the panda in the top-left corner.",
        "在左上角顯示熊貓圖示。",
    ),
    ("Interface size", "介面尺寸"),
    (
        "Adjust text and controls in the sidebar, list and settings.",
        "調整側欄、列表與設定頁的文字及控制項大小。",
    ),
    ("Layout", "版面"),
    ("Collapse feed sidebar", "收合訂閱側欄"),
    ("Leave more room for articles.", "為文章留出更多橫向空間。"),
    ("Collapsed", "已收合"),
    ("Expanded", "已展開"),
    ("Full text", "全文"),
    ("Auto extract full text", "自動擷取全文"),
    (
        "Fetch the original page when opening an article without full content.",
        "開啟尚無全文的文章時，自動擷取原文頁面。",
    ),
    ("On", "開"),
    ("Off", "關"),
    ("Interface text", "介面文字"),
    (
        "Article and interface text follow the interface size.",
        "閱讀區與應用控制項跟隨介面尺寸。",
    ),
    ("Smaller", "縮小"),
    ("Larger", "放大"),
    (
        "Feeds, articles and settings are stored on this device.",
        "訂閱、文章與設定均保存在本機。",
    ),
    ("Papr reference", "Papr 參考"),
    (
        "Papr informed our feed parsing, article extraction, and store design. Panda Reader's implementation is independent and does not include papr-core.",
        "Panda Reader 在訂閱來源解析、正文擷取和 store 設計上參考了 Papr；程式碼為獨立實作，不包含 papr-core。",
    ),
    ("TTY7 reference", "TTY7 參考"),
    (
        "TTY7 inspired our packaging and theme design.",
        "Panda Reader 的打包方式和主題設計參考了 TTY7。",
    ),
    ("Choose a reading palette", "選擇喜歡的閱讀配色"),
    ("Language", "語言"),
    (
        "Choose the language used throughout the app.",
        "選擇應用介面的語言。",
    ),
    ("Slate Workspace", "灰藍 · 工作台"),
    ("Feed URL https://…", "訂閱網址 https://…"),
    ("Search articles", "搜尋文章"),
    (
        "Connecting and syncing Miniflux…",
        "正在連接並同步 Miniflux…",
    ),
    (
        "Disconnected from Miniflux; cached articles remain available",
        "已中斷 Miniflux；本機快取仍可閱讀",
    ),
    ("Adding feed…", "正在新增訂閱…"),
    ("Feed added", "訂閱已新增"),
    ("Removing feed…", "正在刪除訂閱…"),
    (
        "Feed removed from Miniflux and locally",
        "已從 Miniflux 與本機刪除訂閱",
    ),
    ("Syncing Miniflux…", "正在同步 Miniflux…"),
    ("Refreshing local feeds…", "正在重新整理本機訂閱…"),
    ("Extracting full text…", "正在擷取全文…"),
    ("Full text extracted", "已擷取全文"),
    ("OPML exported", "OPML 已匯出"),
    ("Could not save settings: {}", "儲存設定失敗：{}"),
    ("Connected to Miniflux: {}", "已連接 Miniflux：{}"),
    ("Synced {} articles", "已同步 {} 篇文章"),
    ("Imported {} feeds", "已匯入 {} 個訂閱"),
    ("Connected: {}", "已連接：{}"),
    (
        "{} feeds. Additions and removals sync to Miniflux when connected.",
        "{} 個訂閱來源。連接 Miniflux 時，新增與移除會同步到伺服器。",
    ),
    ("{} articles", "{} 篇文章"),
    ("{} unread", "{} 篇未讀"),
    ("Show articles", "查看文章"),
    ("Copy feed title", "複製訂閱名稱"),
    ("Copy feed URL", "複製訂閱網址"),
    ("Open feed URL", "開啟訂閱網址"),
    ("Remove feed", "移除訂閱"),
    ("Open", "開啟"),
    ("Mark as read", "標為已讀"),
    ("Mark as unread", "標為未讀"),
    ("Copy title", "複製標題"),
    ("Copy link", "複製連結"),
];

const JA: &[(&str, &str)] = &[
    ("Choose folder icon", "フォルダーアイコンを選択"),
    ("Folder", "フォルダー"),
    ("Book", "本"),
    ("Globe", "地球"),
    ("Bookmark", "ブックマーク"),
    ("Add feed", "フィードを追加"),
    ("Expand sidebar", "サイドバーを展開"),
    ("Collapse sidebar", "サイドバーを折りたたむ"),
    ("Settings", "設定"),
    ("All Articles", "すべての記事"),
    ("Unread", "未読"),
    ("Starred", "スター付き"),
    ("Star", "スター"),
    ("Read Later", "後で読む"),
    ("Feeds", "フィード"),
    ("Refresh feeds", "フィードを更新"),
    ("Feed", "フィード"),
    ("Loading articles…", "記事を読み込み中…"),
    ("No articles yet", "まだ記事がありません"),
    (
        "Connect Miniflux in Settings to see your articles here",
        "設定で Miniflux に接続すると、ここに記事が表示されます",
    ),
    ("Remove star", "スターを外す"),
    ("Save for later", "後で読む"),
    ("Remove from Read Later", "後で読むから外す"),
    ("Reading", "読書"),
    ("Currently reading", "読書中"),
    ("Extract full text", "全文を取得"),
    ("Re-extract full text", "全文を再取得"),
    ("Hide images", "画像を隠す"),
    ("Show images", "画像を表示"),
    ("Open original", "原文を開く"),
    ("Translate", "翻訳"),
    ("Show original", "原文を表示"),
    ("Show translation", "訳文を表示"),
    ("Translating…", "翻訳中…"),
    ("Translation ready", "翻訳が完了しました"),
    ("Translation", "翻訳"),
    ("Provider", "プロバイダー"),
    ("Azure API key", "Azure API キー"),
    ("Azure region", "Azure リージョン"),
    ("Save translation settings", "翻訳設定を保存"),
    ("Translation settings saved", "翻訳設定を保存しました"),
    (
        "Configure a translation provider in Settings before translating.",
        "翻訳する前に、設定で翻訳プロバイダーを構成してください。",
    ),
    ("Coming soon", "近日対応"),
    (
        "Choose a translation provider for article text.",
        "記事本文の翻訳に使うプロバイダーを選びます。",
    ),
    (
        "Interface language and article translation language can differ.",
        "インターフェース言語と記事の翻訳言語は別に設定できます。",
    ),
    ("Article language", "記事の言語"),
    (
        "Translate article bodies into this language.",
        "記事本文をこの言語に翻訳します。",
    ),
    ("Start reading", "読み始める"),
    (
        "Select an article from the list to begin",
        "リストから記事を選んで読み始めましょう",
    ),
    ("Back to Reader", "リーダーに戻る"),
    ("SETTINGS", "設定"),
    ("General", "一般"),
    ("Reset settings", "設定をリセット"),
    ("Restore defaults", "初期設定に戻す"),
    (
        "Reset app settings now?",
        "アプリの設定をリセットしますか？",
    ),
    (
        "Settings restored to defaults.",
        "設定を初期状態に戻しました。",
    ),
    (
        "Restore interface and translation settings, including saved translation keys, to defaults. Feeds, articles and Provider connections are kept.",
        "保存済みの翻訳キーを含む表示・翻訳設定を初期状態に戻します。フィード、記事、Provider 接続は保持されます。",
    ),
    ("Appearance", "外観"),
    ("About", "情報"),
    ("Development build", "開発ビルド"),
    ("Remove", "削除"),
    (
        "Remove this feed and its local articles? It will also be removed from Miniflux.",
        "このフィードとローカルの記事を削除しますか？Miniflux からも削除されます。",
    ),
    ("Confirm removal", "削除を確認"),
    ("Cancel", "キャンセル"),
    (
        "Connect Miniflux to manage feeds and reading status on the server. Articles remain available offline.",
        "Miniflux に接続すると、フィードと既読状態をサーバーで管理できます。記事はオフラインでも読めます。",
    ),
    ("Server URL", "サーバー URL"),
    ("Update connection", "接続を更新"),
    ("Connect and sync", "接続して同期"),
    ("Disconnect", "切断"),
    ("Feed management", "フィード管理"),
    ("Sync now", "今すぐ同期"),
    ("Import OPML", "OPML をインポート"),
    ("Export OPML", "OPML をエクスポート"),
    ("Added feeds", "追加済みフィード"),
    ("Search added feeds", "追加済みフィードを検索"),
    ("No feeds found", "フィードが見つかりません"),
    ("Theme", "テーマ"),
    (
        "Choose a color theme for reading.",
        "読書用のカラーテーマを選びます。",
    ),
    ("Dark theme", "ダークテーマ"),
    ("Light theme", "ライトテーマ"),
    ("Change theme  ›", "テーマを変更  ›"),
    ("Branding", "ブランド"),
    ("Show panda icon", "パンダのアイコンを表示"),
    (
        "Display the panda in the top-left corner.",
        "左上隅にパンダのアイコンを表示します。",
    ),
    ("Interface size", "インターフェースサイズ"),
    (
        "Adjust text and controls in the sidebar, list and settings.",
        "サイドバー、リスト、設定の文字と操作サイズを調整します。",
    ),
    ("Layout", "レイアウト"),
    ("Collapse feed sidebar", "フィードサイドバーを折りたたむ"),
    (
        "Leave more room for articles.",
        "記事のための横幅を広げます。",
    ),
    ("Collapsed", "折りたたみ"),
    ("Expanded", "展開"),
    ("Full text", "全文"),
    ("Auto extract full text", "全文を自動取得"),
    (
        "Fetch the original page when opening an article without full content.",
        "全文がない記事を開くと、元のページを自動で取得します。",
    ),
    ("On", "オン"),
    ("Off", "オフ"),
    ("Interface text", "インターフェース文字"),
    (
        "Article and interface text follow the interface size.",
        "記事と操作文字はインターフェースサイズに従います。",
    ),
    ("Smaller", "小さく"),
    ("Larger", "大きく"),
    (
        "Feeds, articles and settings are stored on this device.",
        "フィード、記事、設定はこの端末に保存されます。",
    ),
    ("Choose a reading palette", "読書用の配色を選ぶ"),
    ("Language", "言語"),
    (
        "Choose the language used throughout the app.",
        "アプリ全体で使う言語を選びます。",
    ),
    ("Slate Workspace", "スレート・ワークスペース"),
    ("Feed URL https://…", "フィード URL https://…"),
    ("Search articles", "記事を検索"),
    (
        "Connecting and syncing Miniflux…",
        "Miniflux に接続して同期中…",
    ),
    (
        "Disconnected from Miniflux; cached articles remain available",
        "Miniflux から切断しました。キャッシュ記事は読めます",
    ),
    ("Adding feed…", "フィードを追加中…"),
    ("Feed added", "フィードを追加しました"),
    ("Removing feed…", "フィードを削除中…"),
    (
        "Feed removed from Miniflux and locally",
        "Miniflux とローカルからフィードを削除しました",
    ),
    ("Syncing Miniflux…", "Miniflux を同期中…"),
    ("Refreshing local feeds…", "ローカルフィードを更新中…"),
    ("Extracting full text…", "全文を取得中…"),
    ("Full text extracted", "全文を取得しました"),
    ("OPML exported", "OPML を書き出しました"),
    (
        "Could not save settings: {}",
        "設定を保存できませんでした：{}",
    ),
    ("Connected to Miniflux: {}", "Miniflux に接続しました：{}"),
    ("Synced {} articles", "{} 件の記事を同期しました"),
    ("Imported {} feeds", "{} 件のフィードを取り込みました"),
    ("Connected: {}", "接続中：{}"),
    (
        "{} feeds. Additions and removals sync to Miniflux when connected.",
        "{} 件のフィード。接続中は追加と削除が Miniflux に同期されます。",
    ),
    ("{} articles", "{} 件の記事"),
    ("{} unread", "{} 件未読"),
    ("Show articles", "記事を表示"),
    ("Copy feed title", "フィード名をコピー"),
    ("Copy feed URL", "フィード URL をコピー"),
    ("Open feed URL", "フィード URL を開く"),
    ("Remove feed", "フィードを削除"),
    ("Open", "開く"),
    ("Mark as read", "既読にする"),
    ("Mark as unread", "未読にする"),
    ("Copy title", "タイトルをコピー"),
    ("Copy link", "リンクをコピー"),
];

const FR: &[(&str, &str)] = &[
    ("Choose folder icon", "Choisir l’icône du dossier"),
    ("Folder", "Dossier"),
    ("Book", "Livre"),
    ("Globe", "Globe"),
    ("Bookmark", "Signet"),
    ("Add feed", "Ajouter un flux"),
    ("Expand sidebar", "Développer la barre latérale"),
    ("Collapse sidebar", "Réduire la barre latérale"),
    ("Settings", "Réglages"),
    ("All Articles", "Tous les articles"),
    ("Unread", "Non lus"),
    ("Starred", "Favoris"),
    ("Star", "Favori"),
    ("Read Later", "À lire plus tard"),
    ("Feeds", "Flux"),
    ("Refresh feeds", "Actualiser les flux"),
    ("Feed", "Flux"),
    ("Loading articles…", "Chargement des articles…"),
    ("No articles yet", "Aucun article pour le moment"),
    (
        "Connect Miniflux in Settings to see your articles here",
        "Connectez Miniflux dans les réglages pour voir vos articles ici",
    ),
    ("Remove star", "Retirer le favori"),
    ("Save for later", "À lire plus tard"),
    ("Remove from Read Later", "Retirer de À lire plus tard"),
    ("Reading", "Lecture"),
    ("Currently reading", "En cours de lecture"),
    ("Extract full text", "Extraire le texte complet"),
    ("Re-extract full text", "Réextraire le texte complet"),
    ("Hide images", "Masquer les images"),
    ("Show images", "Afficher les images"),
    ("Open original", "Ouvrir l’original"),
    ("Translate", "Traduire"),
    ("Show original", "Afficher l’original"),
    ("Show translation", "Afficher la traduction"),
    ("Translating…", "Traduction…"),
    ("Translation ready", "Traduction prête"),
    ("Translation", "Traduction"),
    ("Provider", "Fournisseur"),
    ("Azure API key", "Clé API Azure"),
    ("Azure region", "Région Azure"),
    (
        "Save translation settings",
        "Enregistrer les réglages de traduction",
    ),
    (
        "Translation settings saved",
        "Réglages de traduction enregistrés",
    ),
    (
        "Configure a translation provider in Settings before translating.",
        "Configurez un fournisseur de traduction dans les réglages avant de traduire.",
    ),
    ("Coming soon", "Bientôt disponible"),
    (
        "Choose a translation provider for article text.",
        "Choisissez un fournisseur pour traduire le texte des articles.",
    ),
    (
        "Interface language and article translation language can differ.",
        "La langue de l’interface et celle de traduction des articles peuvent différer.",
    ),
    ("Article language", "Langue des articles"),
    (
        "Translate article bodies into this language.",
        "Traduire le corps des articles dans cette langue.",
    ),
    ("Start reading", "Commencer à lire"),
    (
        "Select an article from the list to begin",
        "Choisissez un article dans la liste pour commencer",
    ),
    ("Back to Reader", "Retour au lecteur"),
    ("SETTINGS", "RÉGLAGES"),
    ("General", "Général"),
    ("Reset settings", "Réinitialiser les paramètres"),
    ("Restore defaults", "Rétablir les valeurs par défaut"),
    (
        "Reset app settings now?",
        "Réinitialiser les paramètres de l’application ?",
    ),
    (
        "Settings restored to defaults.",
        "Paramètres rétablis par défaut.",
    ),
    (
        "Restore interface and translation settings, including saved translation keys, to defaults. Feeds, articles and Provider connections are kept.",
        "Rétablit les paramètres d’interface et de traduction, y compris les clés enregistrées. Les flux, articles et connexions Provider sont conservés.",
    ),
    ("Appearance", "Apparence"),
    ("About", "À propos"),
    ("Development build", "Build de développement"),
    ("Remove", "Supprimer"),
    (
        "Remove this feed and its local articles? It will also be removed from Miniflux.",
        "Supprimer ce flux et ses articles locaux ? Il sera aussi retiré de Miniflux.",
    ),
    ("Confirm removal", "Confirmer la suppression"),
    ("Cancel", "Annuler"),
    (
        "Connect Miniflux to manage feeds and reading status on the server. Articles remain available offline.",
        "Connectez Miniflux pour gérer les flux et l’état de lecture sur le serveur. Les articles restent disponibles hors ligne.",
    ),
    ("Server URL", "URL du serveur"),
    ("Update connection", "Mettre à jour la connexion"),
    ("Connect and sync", "Connecter et synchroniser"),
    ("Disconnect", "Déconnecter"),
    ("Feed management", "Gestion des flux"),
    ("Sync now", "Synchroniser"),
    ("Import OPML", "Importer OPML"),
    ("Export OPML", "Exporter OPML"),
    ("Added feeds", "Flux ajoutés"),
    ("Search added feeds", "Rechercher dans les flux ajoutés"),
    ("No feeds found", "Aucun flux trouvé"),
    ("Theme", "Thème"),
    (
        "Choose a color theme for reading.",
        "Choisissez un thème de couleurs pour la lecture.",
    ),
    ("Dark theme", "Thème sombre"),
    ("Light theme", "Thème clair"),
    ("Change theme  ›", "Changer de thème  ›"),
    ("Branding", "Identité visuelle"),
    ("Show panda icon", "Afficher l’icône du panda"),
    (
        "Display the panda in the top-left corner.",
        "Afficher le panda dans le coin supérieur gauche.",
    ),
    ("Interface size", "Taille de l’interface"),
    (
        "Adjust text and controls in the sidebar, list and settings.",
        "Ajustez le texte et les contrôles de la barre latérale, de la liste et des réglages.",
    ),
    ("Layout", "Disposition"),
    ("Collapse feed sidebar", "Réduire la barre des flux"),
    (
        "Leave more room for articles.",
        "Laisser plus de place aux articles.",
    ),
    ("Collapsed", "Réduite"),
    ("Expanded", "Développée"),
    ("Full text", "Texte complet"),
    (
        "Auto extract full text",
        "Extraire automatiquement le texte complet",
    ),
    (
        "Fetch the original page when opening an article without full content.",
        "Récupérer la page d’origine à l’ouverture d’un article sans texte complet.",
    ),
    ("On", "Activé"),
    ("Off", "Désactivé"),
    ("Interface text", "Texte de l’interface"),
    (
        "Article and interface text follow the interface size.",
        "Le texte des articles et de l’interface suit la taille choisie.",
    ),
    ("Smaller", "Plus petit"),
    ("Larger", "Plus grand"),
    (
        "Feeds, articles and settings are stored on this device.",
        "Les flux, articles et réglages sont stockés sur cet appareil.",
    ),
    ("Choose a reading palette", "Choisir une palette de lecture"),
    ("Language", "Langue"),
    (
        "Choose the language used throughout the app.",
        "Choisissez la langue de l’application.",
    ),
    ("Slate Workspace", "Espace ardoise"),
    ("Feed URL https://…", "URL du flux https://…"),
    ("Search articles", "Rechercher des articles"),
    (
        "Connecting and syncing Miniflux…",
        "Connexion et synchronisation Miniflux…",
    ),
    (
        "Disconnected from Miniflux; cached articles remain available",
        "Déconnecté de Miniflux ; les articles en cache restent disponibles",
    ),
    ("Adding feed…", "Ajout du flux…"),
    ("Feed added", "Flux ajouté"),
    ("Removing feed…", "Suppression du flux…"),
    (
        "Feed removed from Miniflux and locally",
        "Flux retiré de Miniflux et en local",
    ),
    ("Syncing Miniflux…", "Synchronisation Miniflux…"),
    ("Refreshing local feeds…", "Actualisation des flux locaux…"),
    ("Extracting full text…", "Extraction du texte complet…"),
    ("Full text extracted", "Texte complet extrait"),
    ("OPML exported", "OPML exporté"),
    (
        "Could not save settings: {}",
        "Impossible d’enregistrer les réglages : {}",
    ),
    ("Connected to Miniflux: {}", "Connecté à Miniflux : {}"),
    ("Synced {} articles", "{} articles synchronisés"),
    ("Imported {} feeds", "{} flux importés"),
    ("Connected: {}", "Connecté : {}"),
    (
        "{} feeds. Additions and removals sync to Miniflux when connected.",
        "{} flux. Les ajouts et suppressions se synchronisent avec Miniflux une fois connecté.",
    ),
    ("{} articles", "{} articles"),
    ("{} unread", "{} non lus"),
    ("Show articles", "Afficher les articles"),
    ("Copy feed title", "Copier le titre du flux"),
    ("Copy feed URL", "Copier l’URL du flux"),
    ("Open feed URL", "Ouvrir l’URL du flux"),
    ("Remove feed", "Supprimer le flux"),
    ("Open", "Ouvrir"),
    ("Mark as read", "Marquer comme lu"),
    ("Mark as unread", "Marquer comme non lu"),
    ("Copy title", "Copier le titre"),
    ("Copy link", "Copier le lien"),
];

const DE: &[(&str, &str)] = &[
    ("Choose folder icon", "Ordnersymbol auswählen"),
    ("Folder", "Ordner"),
    ("Book", "Buch"),
    ("Globe", "Globus"),
    ("Bookmark", "Lesezeichen"),
    ("Add feed", "Feed hinzufügen"),
    ("Expand sidebar", "Seitenleiste ausklappen"),
    ("Collapse sidebar", "Seitenleiste einklappen"),
    ("Settings", "Einstellungen"),
    ("All Articles", "Alle Artikel"),
    ("Unread", "Ungelesen"),
    ("Starred", "Markiert"),
    ("Star", "Markieren"),
    ("Read Later", "Später lesen"),
    ("Feeds", "Feeds"),
    ("Refresh feeds", "Feeds aktualisieren"),
    ("Feed", "Feed"),
    ("Loading articles…", "Artikel werden geladen…"),
    ("No articles yet", "Noch keine Artikel"),
    (
        "Connect Miniflux in Settings to see your articles here",
        "Verbinden Sie Miniflux in den Einstellungen, um Artikel hier zu sehen",
    ),
    ("Remove star", "Markierung entfernen"),
    ("Save for later", "Später lesen"),
    ("Remove from Read Later", "Aus Später lesen entfernen"),
    ("Reading", "Lesen"),
    ("Currently reading", "Wird gelesen"),
    ("Extract full text", "Volltext extrahieren"),
    ("Re-extract full text", "Volltext erneut extrahieren"),
    ("Hide images", "Bilder ausblenden"),
    ("Show images", "Bilder anzeigen"),
    ("Open original", "Original öffnen"),
    ("Translate", "Übersetzen"),
    ("Show original", "Original anzeigen"),
    ("Show translation", "Übersetzung anzeigen"),
    ("Translating…", "Wird übersetzt…"),
    ("Translation ready", "Übersetzung bereit"),
    ("Translation", "Übersetzung"),
    ("Provider", "Anbieter"),
    ("Azure API key", "Azure-API-Schlüssel"),
    ("Azure region", "Azure-Region"),
    (
        "Save translation settings",
        "Übersetzungseinstellungen speichern",
    ),
    (
        "Translation settings saved",
        "Übersetzungseinstellungen gespeichert",
    ),
    (
        "Configure a translation provider in Settings before translating.",
        "Konfigurieren Sie vor dem Übersetzen einen Übersetzungsanbieter in den Einstellungen.",
    ),
    ("Coming soon", "Demnächst"),
    (
        "Choose a translation provider for article text.",
        "Wählen Sie einen Anbieter für die Übersetzung von Artikeltext.",
    ),
    (
        "Interface language and article translation language can differ.",
        "Oberflächensprache und Übersetzungssprache für Artikel können sich unterscheiden.",
    ),
    ("Article language", "Artikelsprache"),
    (
        "Translate article bodies into this language.",
        "Artikeltexte in diese Sprache übersetzen.",
    ),
    ("Start reading", "Lesen beginnen"),
    (
        "Select an article from the list to begin",
        "Wählen Sie einen Artikel aus der Liste, um zu beginnen",
    ),
    ("Back to Reader", "Zurück zum Reader"),
    ("SETTINGS", "EINSTELLUNGEN"),
    ("General", "Allgemein"),
    ("Reset settings", "Einstellungen zurücksetzen"),
    ("Restore defaults", "Standardeinstellungen wiederherstellen"),
    (
        "Reset app settings now?",
        "App-Einstellungen jetzt zurücksetzen?",
    ),
    (
        "Settings restored to defaults.",
        "Standardeinstellungen wiederhergestellt.",
    ),
    (
        "Restore interface and translation settings, including saved translation keys, to defaults. Feeds, articles and Provider connections are kept.",
        "Setzt Anzeige- und Übersetzungseinstellungen einschließlich gespeicherter Übersetzungsschlüssel zurück. Feeds, Artikel und Provider-Verbindungen bleiben erhalten.",
    ),
    ("Appearance", "Darstellung"),
    ("About", "Über"),
    ("Development build", "Entwicklungsversion"),
    ("Remove", "Entfernen"),
    (
        "Remove this feed and its local articles? It will also be removed from Miniflux.",
        "Diesen Feed und seine lokalen Artikel entfernen? Er wird auch von Miniflux entfernt.",
    ),
    ("Confirm removal", "Entfernen bestätigen"),
    ("Cancel", "Abbrechen"),
    (
        "Connect Miniflux to manage feeds and reading status on the server. Articles remain available offline.",
        "Verbinden Sie Miniflux, um Feeds und Lesestatus auf dem Server zu verwalten. Artikel bleiben offline verfügbar.",
    ),
    ("Server URL", "Server-URL"),
    ("Update connection", "Verbindung aktualisieren"),
    ("Connect and sync", "Verbinden und synchronisieren"),
    ("Disconnect", "Trennen"),
    ("Feed management", "Feed-Verwaltung"),
    ("Sync now", "Jetzt synchronisieren"),
    ("Import OPML", "OPML importieren"),
    ("Export OPML", "OPML exportieren"),
    ("Added feeds", "Hinzugefügte Feeds"),
    ("Search added feeds", "Hinzugefügte Feeds durchsuchen"),
    ("No feeds found", "Keine Feeds gefunden"),
    ("Theme", "Thema"),
    (
        "Choose a color theme for reading.",
        "Wählen Sie ein Farbschema zum Lesen.",
    ),
    ("Dark theme", "Dunkles Thema"),
    ("Light theme", "Helles Thema"),
    ("Change theme  ›", "Thema ändern  ›"),
    ("Branding", "Markenauftritt"),
    ("Show panda icon", "Panda-Symbol anzeigen"),
    (
        "Display the panda in the top-left corner.",
        "Den Panda oben links anzeigen.",
    ),
    ("Interface size", "Oberflächengröße"),
    (
        "Adjust text and controls in the sidebar, list and settings.",
        "Text und Steuerelemente in Seitenleiste, Liste und Einstellungen anpassen.",
    ),
    ("Layout", "Layout"),
    ("Collapse feed sidebar", "Feed-Seitenleiste einklappen"),
    (
        "Leave more room for articles.",
        "Mehr Platz für Artikel lassen.",
    ),
    ("Collapsed", "Eingeklappt"),
    ("Expanded", "Ausgeklappt"),
    ("Full text", "Volltext"),
    ("Auto extract full text", "Volltext automatisch extrahieren"),
    (
        "Fetch the original page when opening an article without full content.",
        "Beim Öffnen eines Artikels ohne Volltext die Originalseite abrufen.",
    ),
    ("On", "An"),
    ("Off", "Aus"),
    ("Interface text", "Oberflächentext"),
    (
        "Article and interface text follow the interface size.",
        "Artikel- und Oberflächentext folgen der Oberflächengröße.",
    ),
    ("Smaller", "Kleiner"),
    ("Larger", "Größer"),
    (
        "Feeds, articles and settings are stored on this device.",
        "Feeds, Artikel und Einstellungen werden auf diesem Gerät gespeichert.",
    ),
    ("Choose a reading palette", "Lese-Farbpalette wählen"),
    ("Language", "Sprache"),
    (
        "Choose the language used throughout the app.",
        "Wählen Sie die Sprache der App.",
    ),
    ("Slate Workspace", "Schiefer-Arbeitsbereich"),
    ("Feed URL https://…", "Feed-URL https://…"),
    ("Search articles", "Artikel suchen"),
    (
        "Connecting and syncing Miniflux…",
        "Miniflux wird verbunden und synchronisiert…",
    ),
    (
        "Disconnected from Miniflux; cached articles remain available",
        "Von Miniflux getrennt; zwischengespeicherte Artikel bleiben verfügbar",
    ),
    ("Adding feed…", "Feed wird hinzugefügt…"),
    ("Feed added", "Feed hinzugefügt"),
    ("Removing feed…", "Feed wird entfernt…"),
    (
        "Feed removed from Miniflux and locally",
        "Feed von Miniflux und lokal entfernt",
    ),
    ("Syncing Miniflux…", "Miniflux wird synchronisiert…"),
    (
        "Refreshing local feeds…",
        "Lokale Feeds werden aktualisiert…",
    ),
    ("Extracting full text…", "Volltext wird extrahiert…"),
    ("Full text extracted", "Volltext extrahiert"),
    ("OPML exported", "OPML exportiert"),
    (
        "Could not save settings: {}",
        "Einstellungen konnten nicht gespeichert werden: {}",
    ),
    ("Connected to Miniflux: {}", "Mit Miniflux verbunden: {}"),
    ("Synced {} articles", "{} Artikel synchronisiert"),
    ("Imported {} feeds", "{} Feeds importiert"),
    ("Connected: {}", "Verbunden: {}"),
    (
        "{} feeds. Additions and removals sync to Miniflux when connected.",
        "{} Feeds. Hinzufügen und Entfernen werden bei Verbindung mit Miniflux synchronisiert.",
    ),
    ("{} articles", "{} Artikel"),
    ("{} unread", "{} ungelesen"),
    ("Show articles", "Artikel anzeigen"),
    ("Copy feed title", "Feed-Titel kopieren"),
    ("Copy feed URL", "Feed-URL kopieren"),
    ("Open feed URL", "Feed-URL öffnen"),
    ("Remove feed", "Feed entfernen"),
    ("Open", "Öffnen"),
    ("Mark as read", "Als gelesen markieren"),
    ("Mark as unread", "Als ungelesen markieren"),
    ("Copy title", "Titel kopieren"),
    ("Copy link", "Link kopieren"),
];

const ERROR_ZH_CN: &[(&str, &str)] = &[
    ("Background service stopped", "后台服务已关闭"),
    ("Invalid Miniflux URL", "Miniflux 地址无效"),
    (
        "Miniflux URL must use http or https",
        "Miniflux 地址必须使用 http 或 https",
    ),
    ("Enter a Miniflux API token", "请填写 Miniflux API Token"),
    (
        "Miniflux has no available categories",
        "Miniflux 没有可用分类",
    ),
    (
        "Connect Miniflux before removing a remote feed",
        "请先连接 Miniflux，再移除远程订阅",
    ),
    ("initial sync failed", "首次同步失败"),
    ("Feed has no content yet", "订阅暂时没有内容"),
    ("All feeds failed to refresh", "所有订阅刷新失败"),
    (
        "No extractable article content found on this page",
        "网页中没有找到可提取的正文",
    ),
    (
        "Publisher blocked automated article downloads",
        "来源站点拒绝了自动抓取全文",
    ),
    ("Open the original page instead.", "请改用打开原文。"),
    (
        "Could not decode Google News article URL",
        "无法解析谷歌新闻原文链接",
    ),
    (
        "Google News page is missing a decode signature",
        "谷歌新闻页面缺少解析签名",
    ),
    (
        "Google News page is missing a decode timestamp",
        "谷歌新闻页面缺少解析时间戳",
    ),
    (
        "Feed URL must use HTTP or HTTPS",
        "只支持 HTTP 或 HTTPS 订阅地址",
    ),
    (
        "Enter an Azure Translator API key in Settings",
        "请在设置中填写 Azure Translator API 密钥",
    ),
    (
        "Enter Volcengine Access Key ID and Secret Access Key in Settings",
        "请在设置中填写火山引擎 Access Key ID 与 Secret Access Key",
    ),
    ("Nothing to translate", "没有可翻译的内容"),
    ("DeepL is not available yet", "DeepL 尚未支持"),
    (
        "LibreTranslate is not available yet",
        "LibreTranslate 尚未支持",
    ),
];

const ERROR_ZH_TW: &[(&str, &str)] = &[
    ("Background service stopped", "背景服務已關閉"),
    ("Invalid Miniflux URL", "Miniflux 網址無效"),
    (
        "Miniflux URL must use http or https",
        "Miniflux 網址必須使用 http 或 https",
    ),
    ("Enter a Miniflux API token", "請填寫 Miniflux API Token"),
    (
        "Miniflux has no available categories",
        "Miniflux 沒有可用分類",
    ),
    (
        "Connect Miniflux before removing a remote feed",
        "請先連接 Miniflux，再移除遠端訂閱",
    ),
    ("initial sync failed", "首次同步失敗"),
    ("Feed has no content yet", "訂閱暫時沒有內容"),
    ("All feeds failed to refresh", "所有訂閱重新整理失敗"),
    (
        "No extractable article content found on this page",
        "網頁中找不到可擷取的正文",
    ),
    (
        "Publisher blocked automated article downloads",
        "來源網站拒絕了自動擷取全文",
    ),
    ("Open the original page instead.", "請改用開啟原文。"),
    (
        "Could not decode Google News article URL",
        "無法解析 Google 新聞原文連結",
    ),
    (
        "Google News page is missing a decode signature",
        "Google 新聞頁面缺少解析簽章",
    ),
    (
        "Google News page is missing a decode timestamp",
        "Google 新聞頁面缺少解析時間戳",
    ),
    (
        "Feed URL must use HTTP or HTTPS",
        "只支援 HTTP 或 HTTPS 訂閱網址",
    ),
    (
        "Enter an Azure Translator API key in Settings",
        "請在設定中填寫 Azure Translator API 金鑰",
    ),
    ("Nothing to translate", "沒有可翻譯的內容"),
    ("DeepL is not available yet", "DeepL 尚未支援"),
    (
        "LibreTranslate is not available yet",
        "LibreTranslate 尚未支援",
    ),
];

const ERROR_JA: &[(&str, &str)] = &[
    (
        "Background service stopped",
        "バックグラウンドサービスが停止しました",
    ),
    ("Invalid Miniflux URL", "Miniflux URL が無効です"),
    (
        "Miniflux URL must use http or https",
        "Miniflux URL は http または https である必要があります",
    ),
    (
        "Enter a Miniflux API token",
        "Miniflux API トークンを入力してください",
    ),
    (
        "Miniflux has no available categories",
        "Miniflux に利用可能なカテゴリがありません",
    ),
    (
        "Connect Miniflux before removing a remote feed",
        "リモートフィードを削除する前に Miniflux に接続してください",
    ),
    ("initial sync failed", "初回同期に失敗しました"),
    (
        "Feed has no content yet",
        "フィードにはまだ内容がありません",
    ),
    (
        "All feeds failed to refresh",
        "すべてのフィードの更新に失敗しました",
    ),
    (
        "No extractable article content found on this page",
        "このページから抽出できる記事本文が見つかりません",
    ),
    (
        "Publisher blocked automated article downloads",
        "公開元が自動取得を拒否しました",
    ),
    (
        "Open the original page instead.",
        "代わりに原文を開いてください。",
    ),
    (
        "Could not decode Google News article URL",
        "Google ニュースの記事 URL を解析できません",
    ),
    (
        "Google News page is missing a decode signature",
        "Google ニュースのページに解析署名がありません",
    ),
    (
        "Google News page is missing a decode timestamp",
        "Google ニュースのページに解析タイムスタンプがありません",
    ),
    (
        "Feed URL must use HTTP or HTTPS",
        "フィード URL は HTTP または HTTPS のみ対応です",
    ),
    (
        "Enter an Azure Translator API key in Settings",
        "設定で Azure Translator API キーを入力してください",
    ),
    ("Nothing to translate", "翻訳できる内容がありません"),
    ("DeepL is not available yet", "DeepL はまだ利用できません"),
    (
        "LibreTranslate is not available yet",
        "LibreTranslate はまだ利用できません",
    ),
];

const ERROR_FR: &[(&str, &str)] = &[
    (
        "Background service stopped",
        "Service d’arrière-plan arrêté",
    ),
    ("Invalid Miniflux URL", "URL Miniflux invalide"),
    (
        "Miniflux URL must use http or https",
        "L’URL Miniflux doit utiliser http ou https",
    ),
    (
        "Enter a Miniflux API token",
        "Saisissez un jeton d’API Miniflux",
    ),
    (
        "Miniflux has no available categories",
        "Miniflux n’a aucune catégorie disponible",
    ),
    (
        "Connect Miniflux before removing a remote feed",
        "Connectez Miniflux avant de supprimer un flux distant",
    ),
    (
        "initial sync failed",
        "échec de la synchronisation initiale",
    ),
    (
        "Feed has no content yet",
        "Le flux n’a pas encore de contenu",
    ),
    (
        "All feeds failed to refresh",
        "Échec de l’actualisation de tous les flux",
    ),
    (
        "No extractable article content found on this page",
        "Aucun contenu d’article extractible trouvé sur cette page",
    ),
    (
        "Publisher blocked automated article downloads",
        "L’éditeur a bloqué le téléchargement automatique",
    ),
    (
        "Open the original page instead.",
        "Ouvrez plutôt la page originale.",
    ),
    (
        "Could not decode Google News article URL",
        "Impossible de décoder l’URL Google News",
    ),
    (
        "Google News page is missing a decode signature",
        "La page Google News n’a pas de signature de décodage",
    ),
    (
        "Google News page is missing a decode timestamp",
        "La page Google News n’a pas d’horodatage de décodage",
    ),
    (
        "Feed URL must use HTTP or HTTPS",
        "L’URL du flux doit utiliser HTTP ou HTTPS",
    ),
    (
        "Enter an Azure Translator API key in Settings",
        "Saisissez une clé API Azure Translator dans les réglages",
    ),
    ("Nothing to translate", "Rien à traduire"),
    (
        "DeepL is not available yet",
        "DeepL n’est pas encore disponible",
    ),
    (
        "LibreTranslate is not available yet",
        "LibreTranslate n’est pas encore disponible",
    ),
];

const ERROR_DE: &[(&str, &str)] = &[
    ("Background service stopped", "Hintergrunddienst gestoppt"),
    ("Invalid Miniflux URL", "Ungültige Miniflux-URL"),
    (
        "Miniflux URL must use http or https",
        "Miniflux-URL muss http oder https verwenden",
    ),
    (
        "Enter a Miniflux API token",
        "Geben Sie ein Miniflux-API-Token ein",
    ),
    (
        "Miniflux has no available categories",
        "Miniflux hat keine verfügbaren Kategorien",
    ),
    (
        "Connect Miniflux before removing a remote feed",
        "Verbinden Sie Miniflux, bevor Sie einen Remote-Feed entfernen",
    ),
    (
        "initial sync failed",
        "erste Synchronisierung fehlgeschlagen",
    ),
    ("Feed has no content yet", "Feed hat noch keinen Inhalt"),
    (
        "All feeds failed to refresh",
        "Alle Feeds konnten nicht aktualisiert werden",
    ),
    (
        "No extractable article content found on this page",
        "Auf dieser Seite wurde kein extrahierbarer Artikelinhalt gefunden",
    ),
    (
        "Publisher blocked automated article downloads",
        "Herausgeber hat automatische Downloads blockiert",
    ),
    (
        "Open the original page instead.",
        "Öffnen Sie stattdessen die Originalseite.",
    ),
    (
        "Could not decode Google News article URL",
        "Google-News-Artikel-URL konnte nicht dekodiert werden",
    ),
    (
        "Google News page is missing a decode signature",
        "Google-News-Seite fehlt eine Dekodier-Signatur",
    ),
    (
        "Google News page is missing a decode timestamp",
        "Google-News-Seite fehlt ein Dekodier-Zeitstempel",
    ),
    (
        "Feed URL must use HTTP or HTTPS",
        "Feed-URL muss HTTP oder HTTPS verwenden",
    ),
    (
        "Enter an Azure Translator API key in Settings",
        "Geben Sie in den Einstellungen einen Azure-Translator-API-Schlüssel ein",
    ),
    ("Nothing to translate", "Nichts zu übersetzen"),
    (
        "DeepL is not available yet",
        "DeepL ist noch nicht verfügbar",
    ),
    (
        "LibreTranslate is not available yet",
        "LibreTranslate ist noch nicht verfügbar",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Preferences;

    #[test]
    fn existing_preferences_default_to_english_and_language_round_trips() {
        let existing: Preferences = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(existing.language, Language::English);
        assert_eq!(existing.translation_language, Language::ZhCn);

        let mut translated = existing;
        translated.language = Language::ZhCn;
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&translated).unwrap()).unwrap();
        assert_eq!(restored.language, Language::ZhCn);
        assert_eq!(text(restored.language, "Back to Reader"), "返回阅读");
        assert_eq!(text(Language::Japanese, "Settings"), "設定");
        assert_eq!(
            format(Language::French, "Synced {} articles", 12),
            "12 articles synchronisés"
        );
    }
}
