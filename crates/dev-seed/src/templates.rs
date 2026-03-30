use crate::people::{self, LocaleData};
use rand::Rng;

// ── Shared data pools ───────────────────────────────────────

pub static PROJECTS: &[&str] = &[
    "Atlas", "Beacon", "Compass", "Delta", "Echo", "Forge", "Granite",
    "Horizon", "Iris", "Jetstream", "Keystone", "Lighthouse", "Mercury",
    "Nexus", "Orbit", "Pinnacle", "Quantum", "Relay", "Spectrum", "Titan",
];

pub static TEAMS: &[&str] = &[
    "engineering", "platform", "product", "design", "infrastructure",
    "data", "security", "mobile", "frontend", "backend", "devops", "growth",
];

pub static SERVICES: &[&str] = &[
    "Auth Service", "API Gateway", "PostgreSQL", "Redis", "Kubernetes",
    "CloudFront", "Stripe", "Datadog", "PagerDuty", "CircleCI",
    "Elasticsearch", "Kafka", "RabbitMQ", "Terraform", "Vault",
];

pub static TOPICS: &[&str] = &[
    "microservices", "GraphQL", "Rust", "WebAssembly", "machine learning",
    "edge computing", "observability", "TypeScript", "Kubernetes", "CI/CD",
    "database sharding", "caching strategy", "API versioning", "OAuth 2.0",
    "event sourcing", "container security", "performance tuning", "SSO",
    "Italian", "Japanese", "photography", "hiking", "cycling", "cooking",
];

pub static DAYS: &[&str] = &["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];
pub static MONTHS: &[&str] = &[
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];
static STATUSES: &[&str] = &["passed", "failed", "cancelled"];

// ── Category enum ───────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Work,
    Newsletter,
    Commerce,
    Personal,
    Notification,
}

pub static CATEGORIES: &[Category] = &[
    Category::Work,
    Category::Newsletter,
    Category::Commerce,
    Category::Personal,
    Category::Notification,
];

pub static CATEGORY_WEIGHTS: &[f64] = &[0.35, 0.15, 0.10, 0.20, 0.20];

// ── Subject templates (Latin) ───────────────────────────────

fn subject_templates(cat: Category) -> &'static [&'static str] {
    match cat {
        Category::Work => &[
            "Q{q} {year} planning — {team} priorities",
            "Re: Sprint retrospective notes",
            "Updated timeline for {project}",
            "{project}: deployment checklist",
            "Action items from {day}'s standup",
            "RFC: {topic} redesign proposal",
            "Heads up: {service} maintenance window {day}",
            "Interview feedback — {candidate}",
            "[{team}] Weekly sync agenda",
            "Re: Budget approval for {project}",
            "Performance review self-assessment reminder",
            "New hire onboarding — {candidate} starting {day}",
            "Incident postmortem: {service} outage",
            "Re: Migration plan for {service}",
            "Design review: {project} mockups v{v}",
            "OKR check-in — are we on track?",
            "Re: Vendor evaluation — {topic}",
            "Team offsite logistics ({month})",
            "1:1 agenda for {day}",
            "FYI: Policy update — remote work",
        ],
        Category::Newsletter => &[
            "This Week in {topic} — Issue #{n}",
            "{topic} Weekly Digest",
            "The {topic} Newsletter — {month} {year}",
            "[{topic}] What's new this week",
            "Your {month} recap from {service}",
            "\u{1f680} {service} Changelog — {month} {year}",
            "Developer digest: {topic} edition",
            "Industry roundup: {topic} trends",
        ],
        Category::Commerce => &[
            "Your order #{order} has shipped!",
            "Order confirmation — #{order}",
            "Your receipt from {service}",
            "Subscription renewal: {service}",
            "Payment received — Invoice #{n}",
            "Your {service} trial ends in 3 days",
            "Exclusive offer: {pct}% off {topic}",
            "Your monthly statement is ready",
        ],
        Category::Personal => &[
            "Re: Dinner on {day}?",
            "Photos from the trip!",
            "Happy birthday! \u{1f389}",
            "Re: Weekend plans",
            "Check out this article about {topic}",
            "Moving update — new address",
            "Re: Book recommendation",
            "Catching up — it's been a while!",
            "Wedding invitation — save the date",
            "Re: Recipe you asked about",
        ],
        Category::Notification => &[
            "[GitHub] New comment on PR #{n}",
            "[GitHub] {candidate} pushed to {project}",
            "[Jira] {project}-{n}: Status changed to In Review",
            "[Slack] New message in #{team}",
            "Security alert: New sign-in from {topic}",
            "[CI] Build {status} — {project}@main",
            "Calendar: {topic} in 15 minutes",
            "[Sentry] New issue in {service}",
            "Figma: {candidate} commented on {project}",
            "[Linear] {project}-{n} assigned to you",
        ],
    }
}

// ── Body templates (Latin) ──────────────────────────────────

fn body_templates(cat: Category) -> &'static [&'static str] {
    match cat {
        Category::Work => &[
            "<p>Hi team,</p>\n<p>Following up on our discussion from {day}. Here's where we stand:</p>\n<ul>\n<li>The {project} migration is {pct}% complete</li>\n<li>{candidate} is handling the {service} integration</li>\n<li>We need to finalize the {topic} spec by end of week</li>\n</ul>\n<p>Let me know if you have any blockers.</p>\n<p>Best,<br>{sender}</p>",
            "<p>Hey {recipient},</p>\n<p>Just wanted to flag something — the {service} metrics are looking a bit off since {day}'s deploy. Nothing critical, but worth keeping an eye on.</p>\n<p>Dashboard link: <a href=\"#\">{service} monitoring</a></p>\n<p>If it doesn't stabilize by tomorrow, let's roll back.</p>\n<p>— {sender}</p>",
            "<p>All,</p>\n<p>Quick update on {project}:</p>\n<ol>\n<li><strong>Done:</strong> API endpoints, auth flow, basic UI</li>\n<li><strong>In progress:</strong> {topic} implementation ({pct}% done)</li>\n<li><strong>Blocked:</strong> Waiting on {candidate} for the {service} credentials</li>\n</ol>\n<p>ETA for beta: {day}. Let me know if priorities have shifted.</p>\n<p>Thanks,<br>{sender}</p>",
            "<p>Hi {recipient},</p>\n<p>Attaching the revised proposal for the {topic} work. Key changes from v{v}:</p>\n<ul>\n<li>Reduced scope to focus on {service} first</li>\n<li>Updated cost estimates ({pct}% lower than original)</li>\n<li>Added phased rollout plan</li>\n</ul>\n<p>Would love your feedback before I share with the wider team.</p>\n<p>Cheers,<br>{sender}</p>",
        ],
        Category::Newsletter => &[
            "<div style=\"max-width:600px;margin:0 auto;font-family:sans-serif;\">\n<h1 style=\"color:#333;\">This Week in {topic}</h1>\n<p>Here's your weekly roundup of what's happening in the {topic} world.</p>\n<h2>Top Stories</h2>\n<ul>\n<li><strong>Major release:</strong> {service} v{v}.0 brings {topic} support</li>\n<li><strong>Industry news:</strong> {candidate} joins {service} as CTO</li>\n<li><strong>Tutorial:</strong> Getting started with {topic} in 2024</li>\n</ul>\n<hr>\n<p style=\"color:#999;font-size:12px;\">You're receiving this because you subscribed at {service}.com.\n<a href=\"#\">Unsubscribe</a></p>\n</div>",
        ],
        Category::Commerce => &[
            "<div style=\"max-width:600px;margin:0 auto;\">\n<h2>Order Confirmed \u{2713}</h2>\n<p>Thanks for your purchase! Here's your order summary:</p>\n<table style=\"width:100%;border-collapse:collapse;\">\n<tr><td style=\"padding:8px;border-bottom:1px solid #eee;\">{topic}</td><td style=\"text-align:right;\">$49.99</td></tr>\n<tr><td style=\"padding:8px;border-bottom:1px solid #eee;\">Shipping</td><td style=\"text-align:right;\">Free</td></tr>\n<tr><td style=\"padding:8px;font-weight:bold;\">Total</td><td style=\"text-align:right;font-weight:bold;\">$49.99</td></tr>\n</table>\n<p>Order #{order} \u{00b7} Estimated delivery: {day}</p>\n</div>",
        ],
        Category::Personal => &[
            "<p>Hey!</p>\n<p>So good to hear from you. Yeah, {day} works great for dinner. How about that new {topic} place on 5th? I've heard great things.</p>\n<p>Also — did you see {candidate}'s photos from the trip? Absolutely stunning.</p>\n<p>See you {day}!</p>",
            "<p>Hi {recipient},</p>\n<p>I was just reading this article about {topic} and immediately thought of you. The part about {service} is particularly interesting.</p>\n<p>Hope you're doing well! We should catch up soon.</p>\n<p>— {sender}</p>",
        ],
        Category::Notification => &[
            "<div style=\"font-family:monospace;background:#f6f8fa;padding:16px;border-radius:6px;\">\n<p><strong>{candidate}</strong> commented on <a href=\"#\">{project}#{n}</a>:</p>\n<blockquote style=\"border-left:3px solid #ddd;padding-left:12px;color:#555;\">\nLooks good! Just one suggestion — could we add a test for the edge case where {topic} is empty? Otherwise LGTM.\n</blockquote>\n</div>",
            "<div style=\"font-family:sans-serif;\">\n<p>\u{1f534} <strong>Build failed</strong> — {project}@main</p>\n<p>Commit: <code>{order}</code><br>\nAuthor: {candidate}<br>\nFailed step: {service} tests</p>\n<pre style=\"background:#f6f8fa;padding:12px;border-radius:4px;overflow-x:auto;\">\nerror[E0308]: mismatched types\n  --&gt; src/{topic}.rs:42:5\n   |\n42 |     expected_function()\n   |     ^^^^^^^^^^^^^^^^^^^ expected `String`, found `&amp;str`\n</pre>\n</div>",
        ],
    }
}

// ── i18n subject/body templates are stored as parallel arrays ─

/// Get i18n subject templates for a given locale key and category.
/// Falls back to Latin if no locale templates exist.
pub fn i18n_subject_templates(locale_key: &str, cat: Category) -> &'static [&'static str] {
    // The i18n templates are extensive — we store them per locale.
    // For brevity we include the most common (ja, zh, ko); others fall back to Latin.
    match (locale_key, cat) {
        // Japanese
        ("ja", Category::Work) => &[
            "{year}年Q{q} {team}の優先事項について", "Re: スプリント振り返りメモ",
            "{project}のタイムライン更新", "{project}: デプロイチェックリスト",
            "{day}のスタンドアップのアクションアイテム", "RFC: {topic}の再設計提案",
            "{service}メンテナンスのお知らせ（{day}）", "[{team}] 週次ミーティングアジェンダ",
            "{project}のモックアップレビュー v{v}", "1on1アジェンダ（{day}）",
        ],
        ("ja", Category::Newsletter) => &[
            "今週の{topic}ニュース — 第{n}号", "{topic}ウィークリーダイジェスト",
            "{month}の{service}まとめ", "開発者向けダイジェスト: {topic}特集",
        ],
        ("ja", Category::Commerce) => &[
            "ご注文 #{order} が発送されました", "注文確認 — #{order}",
            "{service}からの領収書", "サブスクリプション更新: {service}", "お支払い確認 — 請求書 #{n}",
        ],
        ("ja", Category::Personal) => &[
            "Re: {day}のディナーどう？", "旅行の写真です！", "お誕生日おめでとう！\u{1f389}",
            "Re: 週末の予定", "{topic}についての記事見つけたよ", "引っ越しのお知らせ",
            "Re: おすすめの本", "久しぶり！元気にしてる？",
        ],
        ("ja", Category::Notification) => &[
            "[GitHub] PR #{n}に新しいコメント", "[GitHub] {project}にプッシュされました",
            "[Jira] {project}-{n}: ステータスがレビュー中に変更", "[Slack] #{team}に新しいメッセージ",
            "[CI] ビルド{status} — {project}@main", "[Sentry] {service}で新しい問題が発生",
        ],
        // Chinese
        ("zh", Category::Work) => &[
            "{year}年Q{q} {team}优先级讨论", "回复：冲刺回顾笔记",
            "{project}时间线更新", "{project}：部署检查清单",
            "{day}站会的待办事项", "RFC：{topic}重新设计方案",
            "{service}维护通知（{day}）", "[{team}] 周会议程",
            "{project}设计评审 v{v}", "1对1会议议程（{day}）",
        ],
        ("zh", Category::Newsletter) => &[
            "本周{topic}动态 — 第{n}期", "{topic}周报",
            "{month}{service}月度总结", "开发者周刊：{topic}专题",
        ],
        ("zh", Category::Commerce) => &[
            "您的订单 #{order} 已发货！", "订单确认 — #{order}",
            "来自{service}的收据", "订阅续费：{service}", "付款确认 — 发票 #{n}",
        ],
        ("zh", Category::Personal) => &[
            "回复：{day}一起吃饭？", "旅行照片来啦！", "生日快乐！\u{1f389}",
            "回复：周末计划", "看到一篇关于{topic}的好文章", "搬家通知——新地址",
            "回复：你推荐的那本书", "好久不见！最近怎么样？",
        ],
        ("zh", Category::Notification) => &[
            "[GitHub] PR #{n} 有新评论", "[GitHub] {project}有新的推送",
            "[Jira] {project}-{n}：状态已变更为审核中", "[Slack] #{team}频道有新消息",
            "[CI] 构建{status} — {project}@main", "[Sentry] {service}出现新问题",
        ],
        // Korean
        ("ko", Category::Work) => &[
            "{year}년 Q{q} {team} 우선순위 논의", "Re: 스프린트 회고 노트",
            "{project} 타임라인 업데이트", "{project}: 배포 체크리스트",
            "{day} 스탠드업 액션 아이템", "RFC: {topic} 재설계 제안",
            "{service} 점검 안내 ({day})", "[{team}] 주간 회의 안건",
            "{project} 디자인 리뷰 v{v}", "1:1 미팅 안건 ({day})",
        ],
        ("ko", Category::Newsletter) => &[
            "이번 주 {topic} 소식 — {n}호", "{topic} 주간 다이제스트",
            "{month} {service} 정리", "개발자 다이제스트: {topic} 특집",
        ],
        ("ko", Category::Commerce) => &[
            "주문 #{order} 배송이 시작되었습니다!", "주문 확인 — #{order}",
            "{service} 영수증", "구독 갱신: {service}", "결제 확인 — 청구서 #{n}",
        ],
        ("ko", Category::Personal) => &[
            "Re: {day}에 저녁 어때?", "여행 사진이에요!", "생일 축하해! \u{1f389}",
            "Re: 주말 계획", "{topic}에 대한 기사 봤어?", "이사 알림 — 새 주소",
            "Re: 추천해준 책", "오랜만이야! 잘 지내?",
        ],
        ("ko", Category::Notification) => &[
            "[GitHub] PR #{n}에 새 댓글", "[GitHub] {project}에 새 푸시",
            "[Jira] {project}-{n}: 상태가 리뷰 중으로 변경됨", "[Slack] #{team}에 새 메시지",
            "[CI] 빌드 {status} — {project}@main", "[Sentry] {service}에서 새 이슈 발생",
        ],
        // Arabic
        ("ar", Category::Work) => &[
            "أولويات {team} للربع {q} من {year}", "رد: ملاحظات مراجعة السبرنت",
            "تحديث الجدول الزمني لمشروع {project}", "{project}: قائمة فحص النشر",
            "بنود العمل من اجتماع {day}", "RFC: مقترح إعادة تصميم {topic}",
            "إشعار صيانة {service} ({day})", "[{team}] جدول أعمال الاجتماع الأسبوعي",
            "مراجعة تصميم {project} الإصدار {v}", "جدول أعمال اجتماع 1:1 ({day})",
        ],
        ("ar", Category::Newsletter) => &[
            "أخبار {topic} هذا الأسبوع — العدد {n}", "الملخص الأسبوعي لـ {topic}",
            "ملخص {month} من {service}", "نشرة المطورين: عدد خاص عن {topic}",
        ],
        ("ar", Category::Commerce) => &[
            "تم شحن طلبك #{order}!", "تأكيد الطلب — #{order}",
            "إيصالك من {service}", "تجديد الاشتراك: {service}", "تأكيد الدفع — فاتورة #{n}",
        ],
        ("ar", Category::Personal) => &[
            "رد: عشاء يوم {day}؟", "صور من الرحلة!", "عيد ميلاد سعيد! \u{1f389}",
            "رد: خطط نهاية الأسبوع", "شاهد هذا المقال عن {topic}",
            "تحديث الانتقال — العنوان الجديد", "رد: الكتاب الذي سألت عنه", "وحشتني! كيف حالك؟",
        ],
        ("ar", Category::Notification) => &[
            "[GitHub] تعليق جديد على PR #{n}", "[GitHub] تم الدفع إلى {project}",
            "[Jira] {project}-{n}: تغيرت الحالة إلى قيد المراجعة", "[Slack] رسالة جديدة في #{team}",
            "[CI] البناء {status} — {project}@main", "[Sentry] مشكلة جديدة في {service}",
        ],
        // Russian
        ("ru", Category::Work) => &[
            "Приоритеты {team} на Q{q} {year}", "Re: Заметки с ретроспективы спринта",
            "Обновление сроков по {project}", "{project}: чек-лист деплоя",
            "Экшн-айтемы со стендапа {day}", "RFC: предложение по редизайну {topic}",
            "Уведомление о техработах {service} ({day})", "[{team}] Повестка еженедельной встречи",
            "Ревью дизайна {project} v{v}", "Повестка 1:1 ({day})",
        ],
        ("ru", Category::Newsletter) => &[
            "{topic} на этой неделе — Выпуск #{n}", "Еженедельный дайджест {topic}",
            "Итоги {month} от {service}", "Дайджест разработчика: спецвыпуск {topic}",
        ],
        ("ru", Category::Commerce) => &[
            "Ваш заказ #{order} отправлен!", "Подтверждение заказа — #{order}",
            "Чек от {service}", "Продление подписки: {service}", "Подтверждение оплаты — счёт #{n}",
        ],
        ("ru", Category::Personal) => &[
            "Re: Ужин в {day}?", "Фотки из поездки!", "С днём рождения! \u{1f389}",
            "Re: Планы на выходные", "Глянь статью про {topic}", "Переезд — новый адрес",
            "Re: Книга, которую ты советовал", "Давно не общались! Как дела?",
        ],
        ("ru", Category::Notification) => &[
            "[GitHub] Новый комментарий к PR #{n}", "[GitHub] Пуш в {project}",
            "[Jira] {project}-{n}: Статус изменён на «Ревью»", "[Slack] Новое сообщение в #{team}",
            "[CI] Сборка {status} — {project}@main", "[Sentry] Новая ошибка в {service}",
        ],
        // Hindi
        ("hi", Category::Work) => &[
            "{year} Q{q} {team} की प्राथमिकताएँ", "Re: स्प्रिंट रेट्रोस्पेक्टिव नोट्स",
            "{project} टाइमलाइन अपडेट", "{project}: डिप्लॉयमेंट चेकलिस्ट",
            "{day} स्टैंडअप के एक्शन आइटम्स", "RFC: {topic} रीडिज़ाइन प्रस्ताव",
            "{service} मेंटेनेंस नोटिस ({day})", "[{team}] साप्ताहिक मीटिंग एजेंडा",
            "{project} डिज़ाइन रिव्यू v{v}", "1:1 मीटिंग एजेंडा ({day})",
        ],
        ("hi", Category::Newsletter) => &[
            "इस हफ़्ते {topic} में — अंक #{n}", "{topic} साप्ताहिक डाइजेस्ट",
            "{month} का {service} सारांश", "डेवलपर डाइजेस्ट: {topic} विशेषांक",
        ],
        ("hi", Category::Commerce) => &[
            "आपका ऑर्डर #{order} शिप हो गया है!", "ऑर्डर कन्फ़र्मेशन — #{order}",
            "{service} से रसीद", "सब्सक्रिप्शन रिन्यूअल: {service}", "भुगतान पुष्टि — इनवॉइस #{n}",
        ],
        ("hi", Category::Personal) => &[
            "Re: {day} को डिनर चलें?", "ट्रिप की फ़ोटोज़!", "जन्मदिन मुबारक! \u{1f389}",
            "Re: वीकेंड प्लान", "{topic} पर ये आर्टिकल देखो", "शिफ्टिंग अपडेट — नया पता",
            "Re: तुमने जो किताब बताई थी", "बहुत दिन हो गए! कैसे हो?",
        ],
        ("hi", Category::Notification) => &[
            "[GitHub] PR #{n} पर नया कमेंट", "[GitHub] {project} में नया पुश",
            "[Jira] {project}-{n}: स्टेटस रिव्यू में बदला", "[Slack] #{team} में नया मैसेज",
            "[CI] बिल्ड {status} — {project}@main", "[Sentry] {service} में नई समस्या",
        ],
        // Thai
        ("th", Category::Work) => &[
            "ลำดับความสำคัญ {team} ไตรมาส {q} ปี {year}", "Re: บันทึกการทบทวนสปรินต์",
            "อัปเดตไทม์ไลน์ {project}", "{project}: เช็คลิสต์การ deploy",
            "รายการดำเนินการจากสแตนด์อัป {day}", "RFC: ข้อเสนอรีดีไซน์ {topic}",
            "แจ้งปิดปรับปรุง {service} ({day})", "[{team}] วาระประชุมประจำสัปดาห์",
            "รีวิวดีไซน์ {project} v{v}", "วาระ 1:1 ({day})",
        ],
        ("th", Category::Newsletter) => &[
            "ข่าว {topic} ประจำสัปดาห์ — ฉบับที่ {n}", "สรุป {topic} รายสัปดาห์",
            "สรุป{month}จาก {service}", "สรุปสำหรับนักพัฒนา: พิเศษ {topic}",
        ],
        ("th", Category::Commerce) => &[
            "คำสั่งซื้อ #{order} จัดส่งแล้ว!", "ยืนยันคำสั่งซื้อ — #{order}",
            "ใบเสร็จจาก {service}", "ต่ออายุสมาชิก: {service}", "ยืนยันการชำระเงิน — ใบแจ้งหนี้ #{n}",
        ],
        ("th", Category::Personal) => &[
            "Re: ทานข้าว{day}ไหม?", "รูปจากทริป!", "สุขสันต์วันเกิด! \u{1f389}",
            "Re: แผนสุดสัปดาห์", "เจอบทความเรื่อง{topic}มา", "แจ้งย้ายบ้าน — ที่อยู่ใหม่",
            "Re: หนังสือที่แนะนำ", "ไม่ได้เจอกันนาน! เป็นไงบ้าง?",
        ],
        ("th", Category::Notification) => &[
            "[GitHub] คอมเมนต์ใหม่บน PR #{n}", "[GitHub] push ใหม่ใน {project}",
            "[Jira] {project}-{n}: สถานะเปลี่ยนเป็นรีวิว", "[Slack] ข้อความใหม่ใน #{team}",
            "[CI] บิลด์ {status} — {project}@main", "[Sentry] ปัญหาใหม่ใน {service}",
        ],
        // Fallback to Latin
        _ => subject_templates(cat),
    }
}

/// Get i18n body templates. Returns Latin bodies as fallback.
pub fn i18n_body_templates(locale_key: &str, cat: Category) -> &'static [&'static str] {
    match (locale_key, cat) {
        ("ja", Category::Work) => &["<p>チームの皆さん</p>\n<p>{day}の議論のフォローアップです。現在の状況：</p>\n<ul>\n<li>{project}の移行は{pct}%完了</li>\n<li>{candidate}が{service}の統合を担当</li>\n<li>今週中に{topic}の仕様を確定する必要あり</li>\n</ul>\n<p>ブロッカーがあれば教えてください。</p>\n<p>よろしくお願いします。<br>{sender}</p>"],
        ("ja", Category::Personal) => &["<p>久しぶり！</p>\n<p>うん、{day}のディナーいいね。5丁目の新しい{topic}のお店はどう？評判いいみたいだよ。</p>\n<p>{candidate}の旅行の写真見た？すごくきれいだったね。</p>\n<p>じゃあ{day}に！</p>"],
        ("zh", Category::Work) => &["<p>大家好，</p>\n<p>跟进{day}的讨论，目前进展如下：</p>\n<ul>\n<li>{project}迁移已完成{pct}%</li>\n<li>{candidate}负责{service}的集成工作</li>\n<li>本周需要敲定{topic}的规范</li>\n</ul>\n<p>如有任何阻碍请及时沟通。</p>\n<p>谢谢，<br>{sender}</p>"],
        ("zh", Category::Personal) => &["<p>好久不见！</p>\n<p>嗯，{day}吃饭可以啊。五街那家新开的{topic}店怎么样？听说评价不错。</p>\n<p>你看了{candidate}的旅行照片没？拍得真好。</p>\n<p>{day}见！</p>"],
        ("ko", Category::Work) => &["<p>팀원 여러분,</p>\n<p>{day} 논의 후속 사항입니다. 현재 상황:</p>\n<ul>\n<li>{project} 마이그레이션 {pct}% 완료</li>\n<li>{candidate}님이 {service} 통합 담당</li>\n<li>이번 주까지 {topic} 스펙 확정 필요</li>\n</ul>\n<p>블로커가 있으면 알려주세요.</p>\n<p>감사합니다,<br>{sender}</p>"],
        ("ko", Category::Personal) => &["<p>오랜만이야!</p>\n<p>응, {day} 저녁 좋아. 5번가에 새로 생긴 {topic} 식당 어때? 평이 좋더라고.</p>\n<p>{candidate} 여행 사진 봤어? 진짜 예쁘더라.</p>\n<p>{day}에 보자!</p>"],
        ("ar", Category::Work) => &["<p style=\"direction:rtl;text-align:right;\">مرحباً بالجميع،</p>\n<p style=\"direction:rtl;text-align:right;\">متابعة لنقاشنا يوم {day}. إليكم آخر المستجدات:</p>\n<ul style=\"direction:rtl;text-align:right;\">\n<li>ترحيل {project} مكتمل بنسبة {pct}%</li>\n<li>{candidate} يتولى تكامل {service}</li>\n<li>نحتاج إلى إنهاء مواصفات {topic} بنهاية الأسبوع</li>\n</ul>\n<p style=\"direction:rtl;text-align:right;\">أخبرونا إذا كان لديكم أي عوائق.</p>\n<p style=\"direction:rtl;text-align:right;\">مع أطيب التحيات،<br>{sender}</p>"],
        ("ar", Category::Personal) => &["<p style=\"direction:rtl;text-align:right;\">أهلاً!</p>\n<p style=\"direction:rtl;text-align:right;\">نعم، {day} مناسب للعشاء. ما رأيك بالمطعم الجديد المتخصص في {topic}؟ سمعت أنه ممتاز.</p>\n<p style=\"direction:rtl;text-align:right;\">هل شاهدت صور {candidate} من الرحلة؟ رائعة جداً.</p>\n<p style=\"direction:rtl;text-align:right;\">نراكم {day}!</p>"],
        ("ru", Category::Work) => &["<p>Всем привет,</p>\n<p>По итогам обсуждения в {day}. Текущий статус:</p>\n<ul>\n<li>Миграция {project} завершена на {pct}%</li>\n<li>{candidate} занимается интеграцией с {service}</li>\n<li>Нужно до конца недели утвердить спецификацию {topic}</li>\n</ul>\n<p>Если есть блокеры — сообщите.</p>\n<p>С уважением,<br>{sender}</p>"],
        ("ru", Category::Personal) => &["<p>Привет!</p>\n<p>Да, {day} для ужина отлично подходит. Как насчёт нового ресторана {topic} на Пятой? Говорят, там здорово.</p>\n<p>Видел фотки {candidate} из поездки? Потрясающие.</p>\n<p>До {day}!</p>"],
        ("hi", Category::Work) => &["<p>सभी को नमस्कार,</p>\n<p>{day} की चर्चा पर फ़ॉलो-अप। वर्तमान स्थिति:</p>\n<ul>\n<li>{project} माइग्रेशन {pct}% पूरा</li>\n<li>{candidate} {service} इंटीग्रेशन संभाल रहे हैं</li>\n<li>इस हफ़्ते {topic} स्पेक फ़ाइनल करना ज़रूरी</li>\n</ul>\n<p>कोई ब्लॉकर हो तो बताइए।</p>\n<p>धन्यवाद,<br>{sender}</p>"],
        ("hi", Category::Personal) => &["<p>अरे!</p>\n<p>हाँ, {day} को डिनर के लिए चलते हैं। 5th स्ट्रीट पर नया {topic} रेस्टोरेंट खुला है, कैसा रहेगा? सुना है बहुत अच्छा है।</p>\n<p>{candidate} की ट्रिप फ़ोटोज़ देखीं? बहुत शानदार हैं।</p>\n<p>{day} को मिलते हैं!</p>"],
        ("th", Category::Work) => &["<p>สวัสดีทุกคน</p>\n<p>ติดตามจากการประชุม{day} สถานะปัจจุบัน:</p>\n<ul>\n<li>การย้าย {project} เสร็จ {pct}% แล้ว</li>\n<li>{candidate} ดูแลการเชื่อมต่อ {service}</li>\n<li>ต้องสรุปสเปค {topic} ภายในสัปดาห์นี้</li>\n</ul>\n<p>หากมีปัญหาติดขัดแจ้งได้เลยครับ/ค่ะ</p>\n<p>ขอบคุณครับ/ค่ะ<br>{sender}</p>"],
        ("th", Category::Personal) => &["<p>สวัสดี!</p>\n<p>ได้เลย {day} ทานข้าวกัน ร้าน{topic}ใหม่ที่ซอย 5 ดีไหม? ได้ยินว่าอร่อยมาก</p>\n<p>เห็นรูปทริปของ{candidate}ไหม? สวยมากเลย</p>\n<p>เจอกัน{day}นะ!</p>"],
        // Fallback to Latin
        _ => body_templates(cat),
    }
}

// ── Template filling ────────────────────────────────────────

/// Placeholder values resolved from RNG + data pools.
pub struct FillContext<'a> {
    pub rng: &'a mut dyn RngMut,
    pub locale: Option<&'a LocaleData>,
}

/// Trait to abstract over mutable RNG access in fill context.
pub trait RngMut {
    fn gen_range_usize(&mut self, range: std::ops::Range<usize>) -> usize;
    fn gen_range_i32(&mut self, range: std::ops::Range<i32>) -> i32;
    fn fill_bytes(&mut self, dest: &mut [u8]);
}

impl<R: Rng> RngMut for R {
    fn gen_range_usize(&mut self, range: std::ops::Range<usize>) -> usize {
        self.random_range(range)
    }
    fn gen_range_i32(&mut self, range: std::ops::Range<i32>) -> i32 {
        self.random_range(range)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.fill(dest);
    }
}

fn pick<'a>(rng: &mut dyn RngMut, arr: &'a [&str]) -> &'a str {
    arr[rng.gen_range_usize(0..arr.len())]
}

pub fn fill_template(template: &str, rng: &mut dyn RngMut, locale: Option<&LocaleData>) -> String {
    let (days, months, topics, projects, teams, services) = if let Some(loc) = locale {
        (loc.days, loc.months, loc.topics, loc.projects, loc.teams, loc.services)
    } else {
        (DAYS as &[&str], MONTHS as &[&str], TOPICS as &[&str], PROJECTS as &[&str], TEAMS as &[&str], SERVICES as &[&str])
    };

    let first_names = if let Some(loc) = locale {
        loc.first_names
    } else {
        people::FIRST_NAMES
    };
    let last_names = if let Some(loc) = locale {
        loc.last_names
    } else {
        people::LAST_NAMES
    };

    let q = rng.gen_range_i32(1..5);
    let year = [2024, 2025, 2026][rng.gen_range_usize(0..3)];
    let v = rng.gen_range_i32(1..10);
    let n = rng.gen_range_i32(100..10000);
    let pct = [15, 25, 40, 60, 75, 80, 90, 95][rng.gen_range_usize(0..8)];
    let mut order_bytes = [0u8; 16];
    rng.fill_bytes(&mut order_bytes);
    let order: String = order_bytes[..4].iter().map(|b| format!("{b:02X}")).collect();

    template
        .replace("{q}", &q.to_string())
        .replace("{year}", &year.to_string())
        .replace("{team}", pick(rng, teams))
        .replace("{project}", pick(rng, projects))
        .replace("{day}", pick(rng, days))
        .replace("{topic}", pick(rng, topics))
        .replace("{service}", pick(rng, services))
        .replace("{candidate}", &format!("{} {}", pick(rng, first_names), pick(rng, last_names)))
        .replace("{v}", &v.to_string())
        .replace("{month}", pick(rng, months))
        .replace("{n}", &n.to_string())
        .replace("{order}", &order)
        .replace("{pct}", &pct.to_string())
        .replace("{status}", pick(rng, STATUSES))
        .replace("{sender}", pick(rng, first_names))
        .replace("{recipient}", pick(rng, first_names))
}

pub fn generate_subject(rng: &mut impl Rng, cat: Category, locale: Option<&LocaleData>) -> String {
    let templates = if let Some(loc) = locale {
        i18n_subject_templates(loc.key, cat)
    } else {
        subject_templates(cat)
    };
    let template = templates[rng.random_range(0..templates.len())];
    fill_template(template, rng, locale)
}

pub fn generate_body(rng: &mut impl Rng, cat: Category, locale: Option<&LocaleData>) -> String {
    let templates = if let Some(loc) = locale {
        i18n_body_templates(loc.key, cat)
    } else {
        body_templates(cat)
    };
    let template = templates[rng.random_range(0..templates.len())];
    fill_template(template, rng, locale)
}

pub fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }
    // Collapse whitespace
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}
