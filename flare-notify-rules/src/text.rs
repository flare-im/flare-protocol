//! 通知文案：通用标题与正文、按内容类型的占位、只显示发送者时的正文、撤回、点名标记、
//! 条数前缀与摘要。
//!
//! 推送服务端的兜底文案与引擎自己出的通知用同一套字符串，所以放在规则 crate 里只写一次：
//! 两边各写一份，迟早一边改了另一边没改，同一条消息在后台与前台显示成两种说法。
//!
//! 只有中文与英文两套。语言取设备登记的 BCP 47 标签：主语言子标签是 `zh` 的（`zh`、`zh-CN`、
//! `zh-Hant-TW`、`zh_CN`）用中文，写了别的语言的一律用英文，没写的用中文。
//! 占位的写法与设计组件库的消息摘要一致（`[图片]` / `[Image]`）。
//!
//! 固定文案是 `&'static str`，不分配；带数字的两个辅助函数要拼字符串，用 `alloc`。

use alloc::string::String;

/// 文案语言。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Lang {
    /// 中文。没写语言时也用它。
    #[default]
    Zh,
    /// 英文：写了非中文的任何语言。
    En,
}

impl Lang {
    /// 设备登记的语言标签对应的文案语言。
    ///
    /// 只看主语言子标签（按 `-`、`_`、`.`、`@` 切开后的第一段，不分大小写），
    /// 所以 POSIX 写法的 `zh_CN.UTF-8` 也认得；空白或空串按没写处理。
    pub fn from_locale(tag: &str) -> Self {
        let primary = tag
            .trim()
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or_default();
        if primary.is_empty() || primary.eq_ignore_ascii_case("zh") {
            Self::Zh
        } else {
            Self::En
        }
    }
}

/// 不随内容变化的固定文案。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phrase {
    /// 没有标题、或者不能露出标题时的标题。
    GenericTitle,
    /// 没有内容、或者不能露出内容时的正文（消息）。
    GenericBody,
    /// 同上，通知类内容（好友申请、业务通知）。
    NoticeBody,
    /// 预览设置为「只显示发送者」时的正文，接在标题（发送者或会话）后面读。
    SentMessage,
    /// 撤回替换的正文。
    Recalled,
    /// 点名到接收者时加在正文前的标记。
    MentionMarker,
    /// 图片（含多图）的占位。
    Image,
    /// 视频的占位。
    Video,
    /// 语音的占位。
    Voice,
    /// 文件的占位。
    File,
    /// 位置的占位。
    Location,
    /// 名片（联系人、群）的占位。
    Contact,
    /// 链接卡片的占位。
    Link,
    /// 贴纸的占位。
    Sticker,
    /// 表情的占位。
    Emoji,
    /// 没有纯文本可用的富文本的占位。
    RichText,
    /// 没有专门占位的类型（转发、话题、应用卡片、自定义消息）与空白文本的占位。
    Message,
}

/// 标题前的条数超过它就写成「99+」：再大的数字没有信息量，只会把标题挤出通知栏。
const COUNT_CAP: u32 = 99;

/// 已经写了「被点名」的说法：社交服务端在发送前钩子里盖写的文案可能自带，不再重复加标记。
/// 两种语言的点名标记本身都在里面。
const MENTIONED_MARKS: [&str; 4] = ["有人@你", "有人@我", "[Mentioned you]", "[Mentioned]"];

/// 固定文案。
pub const fn phrase(lang: Lang, phrase: Phrase) -> &'static str {
    match (lang, phrase) {
        (Lang::Zh, Phrase::GenericTitle) => "新消息",
        (Lang::En, Phrase::GenericTitle) => "New message",
        (Lang::Zh, Phrase::GenericBody) => "你收到一条新消息",
        (Lang::En, Phrase::GenericBody) => "You have a new message",
        (Lang::Zh, Phrase::NoticeBody) => "你收到一条通知",
        (Lang::En, Phrase::NoticeBody) => "You have a new notification",
        (Lang::Zh, Phrase::SentMessage) => "发来一条消息",
        (Lang::En, Phrase::SentMessage) => "sent you a message",
        (Lang::Zh, Phrase::Recalled) => "撤回了一条消息",
        (Lang::En, Phrase::Recalled) => "Message recalled",
        (Lang::Zh, Phrase::MentionMarker) => "[有人@你]",
        (Lang::En, Phrase::MentionMarker) => "[Mentioned you]",
        (Lang::Zh, Phrase::Image) => "[图片]",
        (Lang::En, Phrase::Image) => "[Image]",
        (Lang::Zh, Phrase::Video) => "[视频]",
        (Lang::En, Phrase::Video) => "[Video]",
        (Lang::Zh, Phrase::Voice) => "[语音]",
        (Lang::En, Phrase::Voice) => "[Voice]",
        (Lang::Zh, Phrase::File) => "[文件]",
        (Lang::En, Phrase::File) => "[File]",
        (Lang::Zh, Phrase::Location) => "[位置]",
        (Lang::En, Phrase::Location) => "[Location]",
        (Lang::Zh, Phrase::Contact) => "[名片]",
        (Lang::En, Phrase::Contact) => "[Contact]",
        (Lang::Zh, Phrase::Link) => "[链接]",
        (Lang::En, Phrase::Link) => "[Link]",
        (Lang::Zh, Phrase::Sticker) => "[贴纸]",
        (Lang::En, Phrase::Sticker) => "[Sticker]",
        (Lang::Zh, Phrase::Emoji) => "[表情]",
        (Lang::En, Phrase::Emoji) => "[Emoji]",
        (Lang::Zh, Phrase::RichText) => "[富文本]",
        (Lang::En, Phrase::RichText) => "[Rich text]",
        (Lang::Zh, Phrase::Message) => "[消息]",
        (Lang::En, Phrase::Message) => "[Message]",
    }
}

/// 一条通知代表多条未读时标题前的条数（含末尾空格），用户不点开也知道攒了多少。
/// 不到 2 条为 `None`；超过 99 写成「99+」。
pub fn count_prefix(lang: Lang, count: u32) -> Option<String> {
    if count < 2 {
        return None;
    }
    let shown = if count > COUNT_CAP {
        alloc::format!("{COUNT_CAP}+")
    } else {
        alloc::format!("{count}")
    };
    Some(match lang {
        Lang::Zh => alloc::format!("[{shown}条] "),
        Lang::En => alloc::format!("[{shown} messages] "),
    })
}

/// 摘要通知的正文。条数至少是会话数、至少为 1；会话数不到 2 时只说条数。
pub fn digest_text(lang: Lang, conversations: u32, messages: u32) -> String {
    let messages = messages.max(conversations).max(1);
    match lang {
        Lang::Zh if conversations >= 2 => {
            alloc::format!("{conversations} 个会话有 {messages} 条新消息")
        }
        Lang::Zh => alloc::format!("{messages} 条新消息"),
        Lang::En if conversations >= 2 => {
            alloc::format!("{messages} new messages in {conversations} chats")
        }
        Lang::En if messages == 1 => String::from("1 new message"),
        Lang::En => alloc::format!("{messages} new messages"),
    }
}

/// 文字里已经有「被点名」的说法，再加点名标记就重复了。
pub fn mentions_already(text: &str) -> bool {
    MENTIONED_MARKS.iter().any(|mark| text.contains(mark))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHRASES: [Phrase; 6] = [
        Phrase::GenericTitle,
        Phrase::GenericBody,
        Phrase::NoticeBody,
        Phrase::SentMessage,
        Phrase::Recalled,
        Phrase::MentionMarker,
    ];

    const PLACEHOLDERS: [Phrase; 11] = [
        Phrase::Image,
        Phrase::Video,
        Phrase::Voice,
        Phrase::File,
        Phrase::Location,
        Phrase::Contact,
        Phrase::Link,
        Phrase::Sticker,
        Phrase::Emoji,
        Phrase::RichText,
        Phrase::Message,
    ];

    // 固定文案在常量里也能取，引擎可以拿它初始化静态表。
    const TITLE: &str = phrase(Lang::En, Phrase::GenericTitle);

    #[test]
    fn chinese_locales_render_chinese_and_any_other_locale_english() {
        for locale in [
            "",
            "  ",
            "zh",
            "zh-CN",
            "zh-Hans",
            "zh-Hant-TW",
            "ZH-hk",
            "zh_CN",
            "zh_CN.UTF-8",
            "zh@calendar=chinese",
        ] {
            assert_eq!(Lang::from_locale(locale), Lang::Zh, "{locale:?}");
        }
        for locale in ["en", "en-US", "ja-JP", "fr", "de_DE", "zha", "C", "und"] {
            assert_eq!(Lang::from_locale(locale), Lang::En, "{locale:?}");
        }
        assert_eq!(Lang::default(), Lang::Zh);
    }

    #[test]
    fn every_phrase_exists_in_both_languages() {
        for item in PHRASES.into_iter().chain(PLACEHOLDERS) {
            let (zh, en) = (phrase(Lang::Zh, item), phrase(Lang::En, item));
            assert!(!zh.is_empty() && !en.is_empty() && zh != en, "{item:?}");
            assert!(en.is_ascii(), "{item:?}");
        }
        for item in PLACEHOLDERS {
            for text in [phrase(Lang::Zh, item), phrase(Lang::En, item)] {
                assert!(text.starts_with('[') && text.ends_with(']'), "{text}");
            }
        }
        assert_eq!(TITLE, "New message");
        assert_eq!(phrase(Lang::Zh, Phrase::GenericTitle), "新消息");
        assert_eq!(phrase(Lang::Zh, Phrase::Image), "[图片]");
        assert_eq!(phrase(Lang::En, Phrase::RichText), "[Rich text]");
        assert_eq!(phrase(Lang::En, Phrase::SentMessage), "sent you a message");
    }

    #[test]
    fn mention_markers_are_recognised() {
        // 两种语言各自的点名标记都认得出来，正文自带时不会再加一遍。
        assert!(mentions_already(phrase(Lang::Zh, Phrase::MentionMarker)));
        assert!(mentions_already(phrase(Lang::En, Phrase::MentionMarker)));
        assert!(mentions_already("[有人@我] 小明: 开会"));
        assert!(mentions_already("Ann [Mentioned] in Team"));
        assert!(!mentions_already("小明: @张三 开会"));
        assert!(!mentions_already(""));
    }

    #[test]
    fn count_prefix_starts_at_two_and_caps_at_99() {
        for (count, zh, en) in [
            (0, None, None),
            (1, None, None),
            (2, Some("[2条] "), Some("[2 messages] ")),
            (99, Some("[99条] "), Some("[99 messages] ")),
            (100, Some("[99+条] "), Some("[99+ messages] ")),
            (u32::MAX, Some("[99+条] "), Some("[99+ messages] ")),
        ] {
            assert_eq!(count_prefix(Lang::Zh, count).as_deref(), zh, "{count}");
            assert_eq!(count_prefix(Lang::En, count).as_deref(), en, "{count}");
        }
    }

    #[test]
    fn digest_names_conversations_and_messages() {
        assert_eq!(digest_text(Lang::Zh, 7, 42), "7 个会话有 42 条新消息");
        assert_eq!(digest_text(Lang::En, 7, 42), "42 new messages in 7 chats");
        // 会话数不到 2（或没给）时只说条数。
        assert_eq!(digest_text(Lang::Zh, 0, 5), "5 条新消息");
        assert_eq!(digest_text(Lang::Zh, 1, 5), "5 条新消息");
        assert_eq!(digest_text(Lang::En, 1, 5), "5 new messages");
        assert_eq!(digest_text(Lang::En, 0, 0), "1 new message");
        assert_eq!(digest_text(Lang::Zh, 0, 0), "1 条新消息");
        // 条数不会少于会话数。
        assert_eq!(digest_text(Lang::Zh, 3, 1), "3 个会话有 3 条新消息");
        assert_eq!(digest_text(Lang::En, 2, 0), "2 new messages in 2 chats");
    }
}
