#!/usr/bin/env python3
"""
Generate a synthetic Ratatoskr test database with realistic email data.

No external data sources needed — all content is procedurally generated.

Usage:
    python3 generate-test-db.py [output-dir] [--threads N] [--accounts N] [--locale MODE]

Defaults:
    output-dir = ~/.local/share/com.velo.app/
    --threads  = 500
    --accounts = 4
    --locale   = mixed   (mixed | intl | latin)
"""

import sqlite3
import os
import uuid
import random
import re
import argparse
from pathlib import Path
from datetime import datetime, timedelta, timezone

# ── CLI args ─────────────────────────────────────────────────

parser = argparse.ArgumentParser(description="Generate synthetic Ratatoskr test DB")
parser.add_argument("output_dir", nargs="?",
                    default=os.path.expanduser("~/.local/share/com.velo.app"))
parser.add_argument("--threads", type=int, default=500, help="Number of threads")
parser.add_argument("--accounts", type=int, default=4, help="Number of accounts (1-4)")
parser.add_argument("--locale", choices=["mixed", "intl", "latin"], default="mixed",
                    help="mixed=Latin+i18n, intl=non-Latin only, latin=Latin only")
args = parser.parse_args()

OUT_DIR = Path(args.output_dir)
NUM_THREADS = args.threads
NUM_ACCOUNTS = min(max(args.accounts, 1), 4)
LOCALE_MODE = args.locale

OUT_DIR.mkdir(parents=True, exist_ok=True)
OUT_DB = OUT_DIR / "ratatoskr.db"
BODIES_DB = OUT_DIR / "bodies.db"

for db_path in [OUT_DB, BODIES_DB]:
    if db_path.exists():
        print(f"Removing existing {db_path}")
        db_path.unlink()

print(f"Generating {NUM_THREADS} threads across {NUM_ACCOUNTS} account(s) [locale={LOCALE_MODE}]")
print(f"Output: {OUT_DIR}")

# ── Synthetic data pools ─────────────────────────────────────

FIRST_NAMES = [
    "Alice", "Bob", "Carol", "David", "Elena", "Frank", "Grace", "Henry",
    "Iris", "Jack", "Karen", "Leo", "Maya", "Noah", "Olivia", "Paul",
    "Quinn", "Rosa", "Sam", "Tara", "Uma", "Victor", "Wendy", "Xander",
    "Yuki", "Zara", "Amir", "Bianca", "Chen", "Diana", "Erik", "Fatima",
    "George", "Hannah", "Ivan", "Julia", "Kenji", "Lena", "Marco", "Nina",
    "Oscar", "Priya", "Raj", "Sofia", "Tomás", "Ursula", "Wei", "Ximena",
]

LAST_NAMES = [
    "Anderson", "Brown", "Chen", "Davis", "Evans", "Fischer", "Garcia",
    "Hernández", "Ivanova", "Johnson", "Kim", "Lee", "Martinez", "Nakamura",
    "O'Brien", "Patel", "Quinn", "Rossi", "Schmidt", "Taylor", "Ueda",
    "Van den Berg", "Williams", "Xu", "Yamamoto", "Zhang", "Ali", "Björk",
    "Costa", "Dubois", "El-Amin", "Fujimoto", "Gupta", "Hansen", "Ibrahim",
]

DOMAINS = [
    "gmail.com", "outlook.com", "yahoo.com", "protonmail.com",
    "fastmail.com", "hey.com", "icloud.com", "zoho.com",
    "company.io", "startup.dev", "acme.corp", "bigco.com",
    "university.edu", "research.org", "consulting.biz",
]

SUBJECT_TEMPLATES = {
    "work": [
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
    "newsletter": [
        "This Week in {topic} — Issue #{n}",
        "{topic} Weekly Digest",
        "The {topic} Newsletter — {month} {year}",
        "[{topic}] What's new this week",
        "Your {month} recap from {service}",
        "🚀 {service} Changelog — {month} {year}",
        "Developer digest: {topic} edition",
        "Industry roundup: {topic} trends",
    ],
    "commerce": [
        "Your order #{order} has shipped!",
        "Order confirmation — #{order}",
        "Your receipt from {service}",
        "Subscription renewal: {service}",
        "Payment received — Invoice #{n}",
        "Your {service} trial ends in 3 days",
        "Exclusive offer: {pct}% off {topic}",
        "Your monthly statement is ready",
    ],
    "personal": [
        "Re: Dinner on {day}?",
        "Photos from the trip!",
        "Happy birthday! 🎉",
        "Re: Weekend plans",
        "Check out this article about {topic}",
        "Moving update — new address",
        "Re: Book recommendation",
        "Catching up — it's been a while!",
        "Wedding invitation — save the date",
        "Re: Recipe you asked about",
    ],
    "notification": [
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

BODY_TEMPLATES = {
    "work": [
        """<p>Hi team,</p>
<p>Following up on our discussion from {day}. Here's where we stand:</p>
<ul>
<li>The {project} migration is {pct}% complete</li>
<li>{candidate} is handling the {service} integration</li>
<li>We need to finalize the {topic} spec by end of week</li>
</ul>
<p>Let me know if you have any blockers.</p>
<p>Best,<br>{sender}</p>""",
        """<p>Hey {recipient},</p>
<p>Just wanted to flag something — the {service} metrics are looking a bit off since {day}'s deploy. Nothing critical, but worth keeping an eye on.</p>
<p>Dashboard link: <a href="#">{service} monitoring</a></p>
<p>If it doesn't stabilize by tomorrow, let's roll back.</p>
<p>— {sender}</p>""",
        """<p>All,</p>
<p>Quick update on {project}:</p>
<ol>
<li><strong>Done:</strong> API endpoints, auth flow, basic UI</li>
<li><strong>In progress:</strong> {topic} implementation ({pct}% done)</li>
<li><strong>Blocked:</strong> Waiting on {candidate} for the {service} credentials</li>
</ol>
<p>ETA for beta: {day}. Let me know if priorities have shifted.</p>
<p>Thanks,<br>{sender}</p>""",
        """<p>Hi {recipient},</p>
<p>Attaching the revised proposal for the {topic} work. Key changes from v{v}:</p>
<ul>
<li>Reduced scope to focus on {service} first</li>
<li>Updated cost estimates ({pct}% lower than original)</li>
<li>Added phased rollout plan</li>
</ul>
<p>Would love your feedback before I share with the wider team.</p>
<p>Cheers,<br>{sender}</p>""",
    ],
    "newsletter": [
        """<div style="max-width:600px;margin:0 auto;font-family:sans-serif;">
<h1 style="color:#333;">This Week in {topic}</h1>
<p>Here's your weekly roundup of what's happening in the {topic} world.</p>
<h2>Top Stories</h2>
<ul>
<li><strong>Major release:</strong> {service} v{v}.0 brings {topic} support</li>
<li><strong>Industry news:</strong> {candidate} joins {service} as CTO</li>
<li><strong>Tutorial:</strong> Getting started with {topic} in 2024</li>
</ul>
<hr>
<p style="color:#999;font-size:12px;">You're receiving this because you subscribed at {service}.com.
<a href="#">Unsubscribe</a></p>
</div>""",
    ],
    "commerce": [
        """<div style="max-width:600px;margin:0 auto;">
<h2>Order Confirmed ✓</h2>
<p>Thanks for your purchase! Here's your order summary:</p>
<table style="width:100%;border-collapse:collapse;">
<tr><td style="padding:8px;border-bottom:1px solid #eee;">{topic}</td><td style="text-align:right;">$49.99</td></tr>
<tr><td style="padding:8px;border-bottom:1px solid #eee;">Shipping</td><td style="text-align:right;">Free</td></tr>
<tr><td style="padding:8px;font-weight:bold;">Total</td><td style="text-align:right;font-weight:bold;">$49.99</td></tr>
</table>
<p>Order #{order} · Estimated delivery: {day}</p>
</div>""",
    ],
    "personal": [
        """<p>Hey!</p>
<p>So good to hear from you. Yeah, {day} works great for dinner. How about that new {topic} place on 5th? I've heard great things.</p>
<p>Also — did you see {candidate}'s photos from the trip? Absolutely stunning.</p>
<p>See you {day}!</p>""",
        """<p>Hi {recipient},</p>
<p>I was just reading this article about {topic} and immediately thought of you. The part about {service} is particularly interesting.</p>
<p>Hope you're doing well! We should catch up soon.</p>
<p>— {sender}</p>""",
    ],
    "notification": [
        """<div style="font-family:monospace;background:#f6f8fa;padding:16px;border-radius:6px;">
<p><strong>{candidate}</strong> commented on <a href="#">{project}#{n}</a>:</p>
<blockquote style="border-left:3px solid #ddd;padding-left:12px;color:#555;">
Looks good! Just one suggestion — could we add a test for the edge case where {topic} is empty? Otherwise LGTM.
</blockquote>
</div>""",
        """<div style="font-family:sans-serif;">
<p>🔴 <strong>Build failed</strong> — {project}@main</p>
<p>Commit: <code>{order}</code><br>
Author: {candidate}<br>
Failed step: {service} tests</p>
<pre style="background:#f6f8fa;padding:12px;border-radius:4px;overflow-x:auto;">
error[E0308]: mismatched types
  --&gt; src/{topic}.rs:42:5
   |
42 |     expected_function()
   |     ^^^^^^^^^^^^^^^^^^^ expected `String`, found `&amp;str`
</pre>
</div>""",
    ],
}

PROJECTS = [
    "Atlas", "Beacon", "Compass", "Delta", "Echo", "Forge", "Granite",
    "Horizon", "Iris", "Jetstream", "Keystone", "Lighthouse", "Mercury",
    "Nexus", "Orbit", "Pinnacle", "Quantum", "Relay", "Spectrum", "Titan",
]

TEAMS = [
    "engineering", "platform", "product", "design", "infrastructure",
    "data", "security", "mobile", "frontend", "backend", "devops", "growth",
]

SERVICES = [
    "Auth Service", "API Gateway", "PostgreSQL", "Redis", "Kubernetes",
    "CloudFront", "Stripe", "Datadog", "PagerDuty", "CircleCI",
    "Elasticsearch", "Kafka", "RabbitMQ", "Terraform", "Vault",
]

TOPICS = [
    "microservices", "GraphQL", "Rust", "WebAssembly", "machine learning",
    "edge computing", "observability", "TypeScript", "Kubernetes", "CI/CD",
    "database sharding", "caching strategy", "API versioning", "OAuth 2.0",
    "event sourcing", "container security", "performance tuning", "SSO",
    "Italian", "Japanese", "photography", "hiking", "cycling", "cooking",
]

DAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"]
MONTHS = ["January", "February", "March", "April", "May", "June",
          "July", "August", "September", "October", "November", "December"]
STATUSES = ["passed", "failed", "cancelled"]

# ── Non-Latin locale data ─────────────────────────────────────
#
# Each locale has: first_names, last_names, domains, days, months,
# subject_templates (same category keys), body_templates, topics, re_prefix.
# Email addresses use ASCII-safe romanizations.

I18N_LOCALES = {
    "ja": {
        "first_names": [
            "太郎", "花子", "健太", "美咲", "大輔", "由美", "翔太", "さくら",
            "拓也", "陽子", "直樹", "麻衣", "隆", "恵子", "誠", "裕子",
        ],
        "last_names": [
            "田中", "鈴木", "佐藤", "山田", "渡辺", "伊藤", "中村", "小林",
            "加藤", "吉田", "山口", "松本", "高橋", "林", "清水", "藤田",
        ],
        "romanized_first": [
            "taro", "hanako", "kenta", "misaki", "daisuke", "yumi", "shota", "sakura",
            "takuya", "yoko", "naoki", "mai", "takashi", "keiko", "makoto", "yuko",
        ],
        "romanized_last": [
            "tanaka", "suzuki", "sato", "yamada", "watanabe", "ito", "nakamura", "kobayashi",
            "kato", "yoshida", "yamaguchi", "matsumoto", "takahashi", "hayashi", "shimizu", "fujita",
        ],
        "domains": ["gmail.com", "yahoo.co.jp", "docomo.ne.jp", "softbank.ne.jp", "icloud.com"],
        "days": ["月曜日", "火曜日", "水曜日", "木曜日", "金曜日"],
        "months": ["1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月"],
        "re_prefix": "Re:",
        "topics": [
            "マイクロサービス", "クラウド移行", "UI刷新", "セキュリティ監査",
            "パフォーマンス改善", "データベース最適化", "API設計", "テスト自動化",
            "写真", "料理", "旅行", "読書", "キャンプ", "温泉",
        ],
        "projects": ["暁", "富士", "桜", "雷神", "翡翠", "銀河", "鶴", "龍"],
        "teams": ["開発", "設計", "企画", "基盤", "品質管理", "営業"],
        "services": ["認証サービス", "API基盤", "PostgreSQL", "Redis", "監視システム", "CDN"],
        "subject_templates": {
            "work": [
                "{year}年Q{q} {team}の優先事項について",
                "Re: スプリント振り返りメモ",
                "{project}のタイムライン更新",
                "{project}: デプロイチェックリスト",
                "{day}のスタンドアップのアクションアイテム",
                "RFC: {topic}の再設計提案",
                "{service}メンテナンスのお知らせ（{day}）",
                "[{team}] 週次ミーティングアジェンダ",
                "{project}のモックアップレビュー v{v}",
                "1on1アジェンダ（{day}）",
            ],
            "newsletter": [
                "今週の{topic}ニュース — 第{n}号",
                "{topic}ウィークリーダイジェスト",
                "{month}の{service}まとめ",
                "開発者向けダイジェスト: {topic}特集",
            ],
            "commerce": [
                "ご注文 #{order} が発送されました",
                "注文確認 — #{order}",
                "{service}からの領収書",
                "サブスクリプション更新: {service}",
                "お支払い確認 — 請求書 #{n}",
            ],
            "personal": [
                "Re: {day}のディナーどう？",
                "旅行の写真です！",
                "お誕生日おめでとう！🎉",
                "Re: 週末の予定",
                "{topic}についての記事見つけたよ",
                "引っ越しのお知らせ",
                "Re: おすすめの本",
                "久しぶり！元気にしてる？",
            ],
            "notification": [
                "[GitHub] PR #{n}に新しいコメント",
                "[GitHub] {project}にプッシュされました",
                "[Jira] {project}-{n}: ステータスがレビュー中に変更",
                "[Slack] #{team}に新しいメッセージ",
                "[CI] ビルド{status} — {project}@main",
                "[Sentry] {service}で新しい問題が発生",
            ],
        },
        "body_templates": {
            "work": [
                """<p>チームの皆さん</p>
<p>{day}の議論のフォローアップです。現在の状況：</p>
<ul>
<li>{project}の移行は{pct}%完了</li>
<li>{candidate}が{service}の統合を担当</li>
<li>今週中に{topic}の仕様を確定する必要あり</li>
</ul>
<p>ブロッカーがあれば教えてください。</p>
<p>よろしくお願いします。<br>{sender}</p>""",
            ],
            "personal": [
                """<p>久しぶり！</p>
<p>うん、{day}のディナーいいね。5丁目の新しい{topic}のお店はどう？評判いいみたいだよ。</p>
<p>{candidate}の旅行の写真見た？すごくきれいだったね。</p>
<p>じゃあ{day}に！</p>""",
            ],
            "newsletter": [
                """<div style="max-width:600px;margin:0 auto;font-family:sans-serif;">
<h1>今週の{topic}ニュース</h1>
<p>{topic}の最新情報をお届けします。</p>
<h2>トップニュース</h2>
<ul>
<li><strong>メジャーリリース：</strong>{service} v{v}.0が{topic}をサポート</li>
<li><strong>業界ニュース：</strong>{candidate}が{service}のCTOに就任</li>
<li><strong>チュートリアル：</strong>{topic}入門ガイド</li>
</ul>
<hr>
<p style="color:#999;font-size:12px;">{service}.comで購読いただいています。
<a href="#">配信停止</a></p>
</div>""",
            ],
            "commerce": [
                """<div style="max-width:600px;margin:0 auto;">
<h2>ご注文確認 ✓</h2>
<p>ご購入ありがとうございます！ご注文内容：</p>
<table style="width:100%;border-collapse:collapse;">
<tr><td style="padding:8px;border-bottom:1px solid #eee;">{topic}</td><td style="text-align:right;">¥4,980</td></tr>
<tr><td style="padding:8px;border-bottom:1px solid #eee;">送料</td><td style="text-align:right;">無料</td></tr>
<tr><td style="padding:8px;font-weight:bold;">合計</td><td style="text-align:right;font-weight:bold;">¥4,980</td></tr>
</table>
<p>注文番号 #{order} · お届け予定日: {day}</p>
</div>""",
            ],
            "notification": [
                """<div style="font-family:monospace;background:#f6f8fa;padding:16px;border-radius:6px;">
<p><strong>{candidate}</strong>が<a href="#">{project}#{n}</a>にコメントしました：</p>
<blockquote style="border-left:3px solid #ddd;padding-left:12px;color:#555;">
いい感じです！{topic}が空の場合のエッジケーステストを追加できますか？それ以外はLGTMです。
</blockquote>
</div>""",
            ],
        },
    },

    "zh": {
        "first_names": [
            "伟", "芳", "秀英", "敏", "强", "静", "磊", "丽",
            "军", "洋", "艳", "勇", "杰", "娟", "涛", "明",
        ],
        "last_names": [
            "王", "李", "张", "刘", "陈", "杨", "赵", "黄",
            "周", "吴", "徐", "孙", "胡", "朱", "高", "林",
        ],
        "romanized_first": [
            "wei", "fang", "xiuying", "min", "qiang", "jing", "lei", "li",
            "jun", "yang", "yan", "yong", "jie", "juan", "tao", "ming",
        ],
        "romanized_last": [
            "wang", "li", "zhang", "liu", "chen", "yang", "zhao", "huang",
            "zhou", "wu", "xu", "sun", "hu", "zhu", "gao", "lin",
        ],
        "domains": ["gmail.com", "qq.com", "163.com", "126.com", "outlook.com"],
        "days": ["周一", "周二", "周三", "周四", "周五"],
        "months": ["一月", "二月", "三月", "四月", "五月", "六月",
                   "七月", "八月", "九月", "十月", "十一月", "十二月"],
        "re_prefix": "回复：",
        "topics": [
            "微服务架构", "云迁移", "前端重构", "安全审计",
            "性能优化", "数据库优化", "API设计", "自动化测试",
            "摄影", "美食", "旅行", "读书", "健身", "音乐",
        ],
        "projects": ["凤凰", "龙腾", "明月", "长城", "星辰", "大鹏", "翡翠", "麒麟"],
        "teams": ["研发", "设计", "产品", "基础架构", "质量", "运维"],
        "services": ["认证服务", "API网关", "PostgreSQL", "Redis", "监控平台", "消息队列"],
        "subject_templates": {
            "work": [
                "{year}年Q{q} {team}优先级讨论",
                "回复：冲刺回顾笔记",
                "{project}时间线更新",
                "{project}：部署检查清单",
                "{day}站会的待办事项",
                "RFC：{topic}重新设计方案",
                "{service}维护通知（{day}）",
                "[{team}] 周会议程",
                "{project}设计评审 v{v}",
                "1对1会议议程（{day}）",
            ],
            "newsletter": [
                "本周{topic}动态 — 第{n}期",
                "{topic}周报",
                "{month}{service}月度总结",
                "开发者周刊：{topic}专题",
            ],
            "commerce": [
                "您的订单 #{order} 已发货！",
                "订单确认 — #{order}",
                "来自{service}的收据",
                "订阅续费：{service}",
                "付款确认 — 发票 #{n}",
            ],
            "personal": [
                "回复：{day}一起吃饭？",
                "旅行照片来啦！",
                "生日快乐！🎉",
                "回复：周末计划",
                "看到一篇关于{topic}的好文章",
                "搬家通知——新地址",
                "回复：你推荐的那本书",
                "好久不见！最近怎么样？",
            ],
            "notification": [
                "[GitHub] PR #{n} 有新评论",
                "[GitHub] {project}有新的推送",
                "[Jira] {project}-{n}：状态已变更为审核中",
                "[Slack] #{team}频道有新消息",
                "[CI] 构建{status} — {project}@main",
                "[Sentry] {service}出现新问题",
            ],
        },
        "body_templates": {
            "work": [
                """<p>大家好，</p>
<p>跟进{day}的讨论，目前进展如下：</p>
<ul>
<li>{project}迁移已完成{pct}%</li>
<li>{candidate}负责{service}的集成工作</li>
<li>本周需要敲定{topic}的规范</li>
</ul>
<p>如有任何阻碍请及时沟通。</p>
<p>谢谢，<br>{sender}</p>""",
            ],
            "personal": [
                """<p>好久不见！</p>
<p>嗯，{day}吃饭可以啊。五街那家新开的{topic}店怎么样？听说评价不错。</p>
<p>你看了{candidate}的旅行照片没？拍得真好。</p>
<p>{day}见！</p>""",
            ],
            "newsletter": [
                """<div style="max-width:600px;margin:0 auto;font-family:sans-serif;">
<h1>本周{topic}动态</h1>
<p>为您带来{topic}领域的最新资讯。</p>
<h2>热门头条</h2>
<ul>
<li><strong>重大发布：</strong>{service} v{v}.0 支持{topic}</li>
<li><strong>行业动态：</strong>{candidate}加入{service}担任CTO</li>
<li><strong>教程：</strong>{topic}入门指南</li>
</ul>
<hr>
<p style="color:#999;font-size:12px;">您在{service}.com订阅了此邮件。
<a href="#">退订</a></p>
</div>""",
            ],
            "commerce": [
                """<div style="max-width:600px;margin:0 auto;">
<h2>订单确认 ✓</h2>
<p>感谢您的购买！订单详情：</p>
<table style="width:100%;border-collapse:collapse;">
<tr><td style="padding:8px;border-bottom:1px solid #eee;">{topic}</td><td style="text-align:right;">¥329</td></tr>
<tr><td style="padding:8px;border-bottom:1px solid #eee;">运费</td><td style="text-align:right;">免运费</td></tr>
<tr><td style="padding:8px;font-weight:bold;">总计</td><td style="text-align:right;font-weight:bold;">¥329</td></tr>
</table>
<p>订单号 #{order} · 预计送达：{day}</p>
</div>""",
            ],
            "notification": [
                """<div style="font-family:monospace;background:#f6f8fa;padding:16px;border-radius:6px;">
<p><strong>{candidate}</strong>在<a href="#">{project}#{n}</a>上发表了评论：</p>
<blockquote style="border-left:3px solid #ddd;padding-left:12px;color:#555;">
看起来不错！能否为{topic}为空时的边界情况添加一个测试？其他的LGTM。
</blockquote>
</div>""",
            ],
        },
    },

    "ko": {
        "first_names": [
            "민준", "서연", "지호", "하은", "현우", "수빈", "도윤", "지민",
            "서준", "예은", "준서", "다은", "시우", "소율", "유준", "하린",
        ],
        "last_names": [
            "김", "이", "박", "최", "정", "강", "조", "윤",
            "장", "임", "한", "오", "서", "신", "권", "황",
        ],
        "romanized_first": [
            "minjun", "seoyeon", "jiho", "haeun", "hyunwoo", "subin", "doyun", "jimin",
            "seojun", "yeeun", "junseo", "daeun", "siwoo", "soyul", "yujun", "harin",
        ],
        "romanized_last": [
            "kim", "lee", "park", "choi", "jung", "kang", "cho", "yoon",
            "jang", "lim", "han", "oh", "seo", "shin", "kwon", "hwang",
        ],
        "domains": ["gmail.com", "naver.com", "daum.net", "kakao.com", "outlook.com"],
        "days": ["월요일", "화요일", "수요일", "목요일", "금요일"],
        "months": ["1월", "2월", "3월", "4월", "5월", "6월", "7월", "8월", "9월", "10월", "11월", "12월"],
        "re_prefix": "Re:",
        "topics": [
            "마이크로서비스", "클라우드 마이그레이션", "프론트엔드 리팩토링", "보안 감사",
            "성능 최적화", "데이터베이스 최적화", "API 설계", "테스트 자동화",
            "사진", "요리", "여행", "독서", "등산", "음악",
        ],
        "projects": ["무궁화", "한라", "백두", "아리랑", "은하", "태양", "청룡", "봉황"],
        "teams": ["개발", "디자인", "기획", "인프라", "QA", "운영"],
        "services": ["인증 서비스", "API 게이트웨이", "PostgreSQL", "Redis", "모니터링", "메시지 큐"],
        "subject_templates": {
            "work": [
                "{year}년 Q{q} {team} 우선순위 논의",
                "Re: 스프린트 회고 노트",
                "{project} 타임라인 업데이트",
                "{project}: 배포 체크리스트",
                "{day} 스탠드업 액션 아이템",
                "RFC: {topic} 재설계 제안",
                "{service} 점검 안내 ({day})",
                "[{team}] 주간 회의 안건",
                "{project} 디자인 리뷰 v{v}",
                "1:1 미팅 안건 ({day})",
            ],
            "newsletter": [
                "이번 주 {topic} 소식 — {n}호",
                "{topic} 주간 다이제스트",
                "{month} {service} 정리",
                "개발자 다이제스트: {topic} 특집",
            ],
            "commerce": [
                "주문 #{order} 배송이 시작되었습니다!",
                "주문 확인 — #{order}",
                "{service} 영수증",
                "구독 갱신: {service}",
                "결제 확인 — 청구서 #{n}",
            ],
            "personal": [
                "Re: {day}에 저녁 어때?",
                "여행 사진이에요!",
                "생일 축하해! 🎉",
                "Re: 주말 계획",
                "{topic}에 대한 기사 봤어?",
                "이사 알림 — 새 주소",
                "Re: 추천해준 책",
                "오랜만이야! 잘 지내?",
            ],
            "notification": [
                "[GitHub] PR #{n}에 새 댓글",
                "[GitHub] {project}에 새 푸시",
                "[Jira] {project}-{n}: 상태가 리뷰 중으로 변경됨",
                "[Slack] #{team}에 새 메시지",
                "[CI] 빌드 {status} — {project}@main",
                "[Sentry] {service}에서 새 이슈 발생",
            ],
        },
        "body_templates": {
            "work": [
                """<p>팀원 여러분,</p>
<p>{day} 논의 후속 사항입니다. 현재 상황:</p>
<ul>
<li>{project} 마이그레이션 {pct}% 완료</li>
<li>{candidate}님이 {service} 통합 담당</li>
<li>이번 주까지 {topic} 스펙 확정 필요</li>
</ul>
<p>블로커가 있으면 알려주세요.</p>
<p>감사합니다,<br>{sender}</p>""",
            ],
            "personal": [
                """<p>오랜만이야!</p>
<p>응, {day} 저녁 좋아. 5번가에 새로 생긴 {topic} 식당 어때? 평이 좋더라고.</p>
<p>{candidate} 여행 사진 봤어? 진짜 예쁘더라.</p>
<p>{day}에 보자!</p>""",
            ],
            "newsletter": [
                """<div style="max-width:600px;margin:0 auto;font-family:sans-serif;">
<h1>이번 주 {topic} 소식</h1>
<p>{topic} 분야의 최신 소식을 전해드립니다.</p>
<h2>주요 뉴스</h2>
<ul>
<li><strong>메이저 릴리스:</strong> {service} v{v}.0이 {topic} 지원</li>
<li><strong>업계 소식:</strong> {candidate}님이 {service} CTO로 합류</li>
<li><strong>튜토리얼:</strong> {topic} 시작하기</li>
</ul>
<hr>
<p style="color:#999;font-size:12px;">{service}.com에서 구독하셨습니다.
<a href="#">구독 취소</a></p>
</div>""",
            ],
            "commerce": [
                """<div style="max-width:600px;margin:0 auto;">
<h2>주문 확인 ✓</h2>
<p>구매해 주셔서 감사합니다! 주문 내역:</p>
<table style="width:100%;border-collapse:collapse;">
<tr><td style="padding:8px;border-bottom:1px solid #eee;">{topic}</td><td style="text-align:right;">₩59,000</td></tr>
<tr><td style="padding:8px;border-bottom:1px solid #eee;">배송비</td><td style="text-align:right;">무료</td></tr>
<tr><td style="padding:8px;font-weight:bold;">합계</td><td style="text-align:right;font-weight:bold;">₩59,000</td></tr>
</table>
<p>주문번호 #{order} · 배송 예정일: {day}</p>
</div>""",
            ],
            "notification": [
                """<div style="font-family:monospace;background:#f6f8fa;padding:16px;border-radius:6px;">
<p><strong>{candidate}</strong>님이 <a href="#">{project}#{n}</a>에 댓글을 남겼습니다:</p>
<blockquote style="border-left:3px solid #ddd;padding-left:12px;color:#555;">
좋아 보입니다! {topic}이 비어있을 때의 엣지 케이스 테스트를 추가할 수 있을까요? 나머지는 LGTM입니다.
</blockquote>
</div>""",
            ],
        },
    },

    "ar": {
        "first_names": [
            "محمد", "فاطمة", "أحمد", "عائشة", "علي", "مريم", "عمر", "نور",
            "خالد", "سارة", "يوسف", "زينب", "حسن", "ليلى", "إبراهيم", "هدى",
        ],
        "last_names": [
            "الأحمد", "المحمد", "العلي", "الحسن", "الخالد", "الرشيد", "السعيد", "الكريم",
            "الحمد", "الناصر", "العمر", "الصالح", "القاسم", "الفهد", "المنصور", "الهاشمي",
        ],
        "romanized_first": [
            "mohammed", "fatima", "ahmed", "aisha", "ali", "maryam", "omar", "noor",
            "khaled", "sarah", "youssef", "zainab", "hassan", "layla", "ibrahim", "huda",
        ],
        "romanized_last": [
            "alahmad", "almohammed", "alali", "alhassan", "alkhaled", "alrashid", "alsaid", "alkarim",
            "alhamd", "alnasser", "alomar", "alsaleh", "alqasim", "alfahd", "almansour", "alhashimi",
        ],
        "domains": ["gmail.com", "outlook.com", "yahoo.com", "hotmail.com", "outlook.sa"],
        "days": ["الإثنين", "الثلاثاء", "الأربعاء", "الخميس", "الجمعة"],
        "months": ["يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو",
                   "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر"],
        "re_prefix": "رد:",
        "topics": [
            "الخدمات المصغرة", "الترحيل السحابي", "إعادة هيكلة الواجهة", "التدقيق الأمني",
            "تحسين الأداء", "تحسين قاعدة البيانات", "تصميم API", "أتمتة الاختبارات",
            "التصوير", "الطبخ", "السفر", "القراءة", "الرياضة", "الموسيقى",
        ],
        "projects": ["الفلك", "النجم", "البرق", "الصقر", "الواحة", "القمر", "السيف", "النخلة"],
        "teams": ["التطوير", "التصميم", "المنتج", "البنية التحتية", "الجودة", "العمليات"],
        "services": ["خدمة المصادقة", "بوابة API", "PostgreSQL", "Redis", "نظام المراقبة", "طابور الرسائل"],
        "subject_templates": {
            "work": [
                "أولويات {team} للربع {q} من {year}",
                "رد: ملاحظات مراجعة السبرنت",
                "تحديث الجدول الزمني لمشروع {project}",
                "{project}: قائمة فحص النشر",
                "بنود العمل من اجتماع {day}",
                "RFC: مقترح إعادة تصميم {topic}",
                "إشعار صيانة {service} ({day})",
                "[{team}] جدول أعمال الاجتماع الأسبوعي",
                "مراجعة تصميم {project} الإصدار {v}",
                "جدول أعمال اجتماع 1:1 ({day})",
            ],
            "newsletter": [
                "أخبار {topic} هذا الأسبوع — العدد {n}",
                "الملخص الأسبوعي لـ {topic}",
                "ملخص {month} من {service}",
                "نشرة المطورين: عدد خاص عن {topic}",
            ],
            "commerce": [
                "تم شحن طلبك #{order}!",
                "تأكيد الطلب — #{order}",
                "إيصالك من {service}",
                "تجديد الاشتراك: {service}",
                "تأكيد الدفع — فاتورة #{n}",
            ],
            "personal": [
                "رد: عشاء يوم {day}؟",
                "صور من الرحلة!",
                "عيد ميلاد سعيد! 🎉",
                "رد: خطط نهاية الأسبوع",
                "شاهد هذا المقال عن {topic}",
                "تحديث الانتقال — العنوان الجديد",
                "رد: الكتاب الذي سألت عنه",
                "وحشتني! كيف حالك؟",
            ],
            "notification": [
                "[GitHub] تعليق جديد على PR #{n}",
                "[GitHub] تم الدفع إلى {project}",
                "[Jira] {project}-{n}: تغيرت الحالة إلى قيد المراجعة",
                "[Slack] رسالة جديدة في #{team}",
                "[CI] البناء {status} — {project}@main",
                "[Sentry] مشكلة جديدة في {service}",
            ],
        },
        "body_templates": {
            "work": [
                """<p style="direction:rtl;text-align:right;">مرحباً بالجميع،</p>
<p style="direction:rtl;text-align:right;">متابعة لنقاشنا يوم {day}. إليكم آخر المستجدات:</p>
<ul style="direction:rtl;text-align:right;">
<li>ترحيل {project} مكتمل بنسبة {pct}%</li>
<li>{candidate} يتولى تكامل {service}</li>
<li>نحتاج إلى إنهاء مواصفات {topic} بنهاية الأسبوع</li>
</ul>
<p style="direction:rtl;text-align:right;">أخبرونا إذا كان لديكم أي عوائق.</p>
<p style="direction:rtl;text-align:right;">مع أطيب التحيات،<br>{sender}</p>""",
            ],
            "personal": [
                """<p style="direction:rtl;text-align:right;">أهلاً!</p>
<p style="direction:rtl;text-align:right;">نعم، {day} مناسب للعشاء. ما رأيك بالمطعم الجديد المتخصص في {topic}؟ سمعت أنه ممتاز.</p>
<p style="direction:rtl;text-align:right;">هل شاهدت صور {candidate} من الرحلة؟ رائعة جداً.</p>
<p style="direction:rtl;text-align:right;">نراكم {day}!</p>""",
            ],
            "newsletter": [
                """<div style="max-width:600px;margin:0 auto;font-family:sans-serif;direction:rtl;text-align:right;">
<h1>أخبار {topic} هذا الأسبوع</h1>
<p>إليكم أحدث الأخبار في عالم {topic}.</p>
<h2>أبرز الأخبار</h2>
<ul>
<li><strong>إصدار رئيسي:</strong> {service} الإصدار {v}.0 يدعم {topic}</li>
<li><strong>أخبار الصناعة:</strong> {candidate} ينضم إلى {service} كمدير تقني</li>
<li><strong>دليل تعليمي:</strong> البدء مع {topic}</li>
</ul>
<hr>
<p style="color:#999;font-size:12px;">تلقيت هذا البريد لأنك مشترك في {service}.com.
<a href="#">إلغاء الاشتراك</a></p>
</div>""",
            ],
            "commerce": [
                """<div style="max-width:600px;margin:0 auto;direction:rtl;text-align:right;">
<h2>تأكيد الطلب ✓</h2>
<p>شكراً لشرائك! ملخص الطلب:</p>
<table style="width:100%;border-collapse:collapse;">
<tr><td style="padding:8px;border-bottom:1px solid #eee;">{topic}</td><td style="text-align:left;">199 ر.س</td></tr>
<tr><td style="padding:8px;border-bottom:1px solid #eee;">الشحن</td><td style="text-align:left;">مجاني</td></tr>
<tr><td style="padding:8px;font-weight:bold;">المجموع</td><td style="text-align:left;font-weight:bold;">199 ر.س</td></tr>
</table>
<p>رقم الطلب #{order} · التسليم المتوقع: {day}</p>
</div>""",
            ],
            "notification": [
                """<div style="font-family:monospace;background:#f6f8fa;padding:16px;border-radius:6px;direction:rtl;text-align:right;">
<p><strong>{candidate}</strong> علّق على <a href="#">{project}#{n}</a>:</p>
<blockquote style="border-right:3px solid #ddd;padding-right:12px;color:#555;border-left:none;">
يبدو جيداً! هل يمكن إضافة اختبار للحالة الحدية عندما يكون {topic} فارغاً؟ عدا ذلك LGTM.
</blockquote>
</div>""",
            ],
        },
    },

    "ru": {
        "first_names": [
            "Александр", "Мария", "Дмитрий", "Анна", "Сергей", "Елена", "Андрей", "Ольга",
            "Иван", "Наталья", "Михаил", "Татьяна", "Николай", "Екатерина", "Павел", "Светлана",
        ],
        "last_names": [
            "Иванов", "Петрова", "Сидоров", "Козлова", "Новиков", "Морозова", "Волков", "Соколова",
            "Кузнецов", "Попова", "Лебедев", "Смирнова", "Фёдоров", "Васильева", "Орлов", "Никитина",
        ],
        "romanized_first": [
            "alexander", "maria", "dmitry", "anna", "sergey", "elena", "andrey", "olga",
            "ivan", "natalya", "mikhail", "tatyana", "nikolay", "ekaterina", "pavel", "svetlana",
        ],
        "romanized_last": [
            "ivanov", "petrova", "sidorov", "kozlova", "novikov", "morozova", "volkov", "sokolova",
            "kuznetsov", "popova", "lebedev", "smirnova", "fedorov", "vasileva", "orlov", "nikitina",
        ],
        "domains": ["gmail.com", "yandex.ru", "mail.ru", "rambler.ru", "outlook.com"],
        "days": ["понедельник", "вторник", "среда", "четверг", "пятница"],
        "months": ["январь", "февраль", "март", "апрель", "май", "июнь",
                   "июль", "август", "сентябрь", "октябрь", "ноябрь", "декабрь"],
        "re_prefix": "Re:",
        "topics": [
            "микросервисы", "облачная миграция", "рефакторинг фронтенда", "аудит безопасности",
            "оптимизация производительности", "оптимизация БД", "проектирование API", "автотесты",
            "фотография", "кулинария", "путешествия", "чтение", "спорт", "музыка",
        ],
        "projects": ["Буран", "Спутник", "Тайга", "Байкал", "Восток", "Сокол", "Аврора", "Кедр"],
        "teams": ["разработка", "дизайн", "продукт", "инфраструктура", "QA", "DevOps"],
        "services": ["сервис аутентификации", "API-шлюз", "PostgreSQL", "Redis", "мониторинг", "Kafka"],
        "subject_templates": {
            "work": [
                "Приоритеты {team} на Q{q} {year}",
                "Re: Заметки с ретроспективы спринта",
                "Обновление сроков по {project}",
                "{project}: чек-лист деплоя",
                "Экшн-айтемы со стендапа {day}",
                "RFC: предложение по редизайну {topic}",
                "Уведомление о техработах {service} ({day})",
                "[{team}] Повестка еженедельной встречи",
                "Ревью дизайна {project} v{v}",
                "Повестка 1:1 ({day})",
            ],
            "newsletter": [
                "{topic} на этой неделе — Выпуск #{n}",
                "Еженедельный дайджест {topic}",
                "Итоги {month} от {service}",
                "Дайджест разработчика: спецвыпуск {topic}",
            ],
            "commerce": [
                "Ваш заказ #{order} отправлен!",
                "Подтверждение заказа — #{order}",
                "Чек от {service}",
                "Продление подписки: {service}",
                "Подтверждение оплаты — счёт #{n}",
            ],
            "personal": [
                "Re: Ужин в {day}?",
                "Фотки из поездки!",
                "С днём рождения! 🎉",
                "Re: Планы на выходные",
                "Глянь статью про {topic}",
                "Переезд — новый адрес",
                "Re: Книга, которую ты советовал",
                "Давно не общались! Как дела?",
            ],
            "notification": [
                "[GitHub] Новый комментарий к PR #{n}",
                "[GitHub] Пуш в {project}",
                "[Jira] {project}-{n}: Статус изменён на «Ревью»",
                "[Slack] Новое сообщение в #{team}",
                "[CI] Сборка {status} — {project}@main",
                "[Sentry] Новая ошибка в {service}",
            ],
        },
        "body_templates": {
            "work": [
                """<p>Всем привет,</p>
<p>По итогам обсуждения в {day}. Текущий статус:</p>
<ul>
<li>Миграция {project} завершена на {pct}%</li>
<li>{candidate} занимается интеграцией с {service}</li>
<li>Нужно до конца недели утвердить спецификацию {topic}</li>
</ul>
<p>Если есть блокеры — сообщите.</p>
<p>С уважением,<br>{sender}</p>""",
            ],
            "personal": [
                """<p>Привет!</p>
<p>Да, {day} для ужина отлично подходит. Как насчёт нового ресторана {topic} на Пятой? Говорят, там здорово.</p>
<p>Видел фотки {candidate} из поездки? Потрясающие.</p>
<p>До {day}!</p>""",
            ],
            "newsletter": [
                """<div style="max-width:600px;margin:0 auto;font-family:sans-serif;">
<h1>{topic} на этой неделе</h1>
<p>Еженедельная подборка новостей из мира {topic}.</p>
<h2>Главное</h2>
<ul>
<li><strong>Крупный релиз:</strong> {service} v{v}.0 с поддержкой {topic}</li>
<li><strong>Индустрия:</strong> {candidate} — новый CTO {service}</li>
<li><strong>Туториал:</strong> Начало работы с {topic}</li>
</ul>
<hr>
<p style="color:#999;font-size:12px;">Вы подписаны на рассылку {service}.com.
<a href="#">Отписаться</a></p>
</div>""",
            ],
            "commerce": [
                """<div style="max-width:600px;margin:0 auto;">
<h2>Заказ подтверждён ✓</h2>
<p>Спасибо за покупку! Детали заказа:</p>
<table style="width:100%;border-collapse:collapse;">
<tr><td style="padding:8px;border-bottom:1px solid #eee;">{topic}</td><td style="text-align:right;">4 990 ₽</td></tr>
<tr><td style="padding:8px;border-bottom:1px solid #eee;">Доставка</td><td style="text-align:right;">Бесплатно</td></tr>
<tr><td style="padding:8px;font-weight:bold;">Итого</td><td style="text-align:right;font-weight:bold;">4 990 ₽</td></tr>
</table>
<p>Заказ #{order} · Ожидаемая доставка: {day}</p>
</div>""",
            ],
            "notification": [
                """<div style="font-family:monospace;background:#f6f8fa;padding:16px;border-radius:6px;">
<p><strong>{candidate}</strong> прокомментировал <a href="#">{project}#{n}</a>:</p>
<blockquote style="border-left:3px solid #ddd;padding-left:12px;color:#555;">
Выглядит хорошо! Можно добавить тест на граничный случай, когда {topic} пустой? В остальном LGTM.
</blockquote>
</div>""",
            ],
        },
    },

    "hi": {
        "first_names": [
            "आदित्य", "प्रिया", "राहुल", "अनीता", "विकास", "सुनीता", "अमित", "नेहा",
            "रोहित", "पूजा", "संजय", "कविता", "मनीष", "रीना", "दीपक", "स्वाति",
        ],
        "last_names": [
            "शर्मा", "वर्मा", "सिंह", "गुप्ता", "कुमार", "पटेल", "जोशी", "मिश्रा",
            "अग्रवाल", "राव", "चौहान", "यादव", "तिवारी", "पांडे", "दुबे", "श्रीवास्तव",
        ],
        "romanized_first": [
            "aditya", "priya", "rahul", "anita", "vikas", "sunita", "amit", "neha",
            "rohit", "pooja", "sanjay", "kavita", "manish", "reena", "deepak", "swati",
        ],
        "romanized_last": [
            "sharma", "verma", "singh", "gupta", "kumar", "patel", "joshi", "mishra",
            "agrawal", "rao", "chauhan", "yadav", "tiwari", "pandey", "dubey", "srivastava",
        ],
        "domains": ["gmail.com", "yahoo.co.in", "rediffmail.com", "outlook.com", "hotmail.com"],
        "days": ["सोमवार", "मंगलवार", "बुधवार", "गुरुवार", "शुक्रवार"],
        "months": ["जनवरी", "फ़रवरी", "मार्च", "अप्रैल", "मई", "जून",
                   "जुलाई", "अगस्त", "सितम्बर", "अक्टूबर", "नवम्बर", "दिसम्बर"],
        "re_prefix": "Re:",
        "topics": [
            "माइक्रोसर्विसेज़", "क्लाउड माइग्रेशन", "फ्रंटएंड रीफैक्टरिंग", "सुरक्षा ऑडिट",
            "परफ़ॉर्मेंस ऑप्टिमाइज़ेशन", "डेटाबेस ऑप्टिमाइज़ेशन", "API डिज़ाइन", "टेस्ट ऑटोमेशन",
            "फ़ोटोग्राफ़ी", "खाना पकाना", "यात्रा", "पढ़ाई", "क्रिकेट", "संगीत",
        ],
        "projects": ["गरुड़", "हिमालय", "चक्र", "वज्र", "सूर्य", "गंगा", "अग्नि", "इंद्र"],
        "teams": ["डेवलपमेंट", "डिज़ाइन", "प्रोडक्ट", "इंफ्रास्ट्रक्चर", "क्वालिटी", "ऑपरेशंस"],
        "services": ["ऑथ सर्विस", "API गेटवे", "PostgreSQL", "Redis", "मॉनिटरिंग", "मैसेज क्यू"],
        "subject_templates": {
            "work": [
                "{year} Q{q} {team} की प्राथमिकताएँ",
                "Re: स्प्रिंट रेट्रोस्पेक्टिव नोट्स",
                "{project} टाइमलाइन अपडेट",
                "{project}: डिप्लॉयमेंट चेकलिस्ट",
                "{day} स्टैंडअप के एक्शन आइटम्स",
                "RFC: {topic} रीडिज़ाइन प्रस्ताव",
                "{service} मेंटेनेंस नोटिस ({day})",
                "[{team}] साप्ताहिक मीटिंग एजेंडा",
                "{project} डिज़ाइन रिव्यू v{v}",
                "1:1 मीटिंग एजेंडा ({day})",
            ],
            "newsletter": [
                "इस हफ़्ते {topic} में — अंक #{n}",
                "{topic} साप्ताहिक डाइजेस्ट",
                "{month} का {service} सारांश",
                "डेवलपर डाइजेस्ट: {topic} विशेषांक",
            ],
            "commerce": [
                "आपका ऑर्डर #{order} शिप हो गया है!",
                "ऑर्डर कन्फ़र्मेशन — #{order}",
                "{service} से रसीद",
                "सब्सक्रिप्शन रिन्यूअल: {service}",
                "भुगतान पुष्टि — इनवॉइस #{n}",
            ],
            "personal": [
                "Re: {day} को डिनर चलें?",
                "ट्रिप की फ़ोटोज़!",
                "जन्मदिन मुबारक! 🎉",
                "Re: वीकेंड प्लान",
                "{topic} पर ये आर्टिकल देखो",
                "शिफ्टिंग अपडेट — नया पता",
                "Re: तुमने जो किताब बताई थी",
                "बहुत दिन हो गए! कैसे हो?",
            ],
            "notification": [
                "[GitHub] PR #{n} पर नया कमेंट",
                "[GitHub] {project} में नया पुश",
                "[Jira] {project}-{n}: स्टेटस रिव्यू में बदला",
                "[Slack] #{team} में नया मैसेज",
                "[CI] बिल्ड {status} — {project}@main",
                "[Sentry] {service} में नई समस्या",
            ],
        },
        "body_templates": {
            "work": [
                """<p>सभी को नमस्कार,</p>
<p>{day} की चर्चा पर फ़ॉलो-अप। वर्तमान स्थिति:</p>
<ul>
<li>{project} माइग्रेशन {pct}% पूरा</li>
<li>{candidate} {service} इंटीग्रेशन संभाल रहे हैं</li>
<li>इस हफ़्ते {topic} स्पेक फ़ाइनल करना ज़रूरी</li>
</ul>
<p>कोई ब्लॉकर हो तो बताइए।</p>
<p>धन्यवाद,<br>{sender}</p>""",
            ],
            "personal": [
                """<p>अरे!</p>
<p>हाँ, {day} को डिनर के लिए चलते हैं। 5th स्ट्रीट पर नया {topic} रेस्टोरेंट खुला है, कैसा रहेगा? सुना है बहुत अच्छा है।</p>
<p>{candidate} की ट्रिप फ़ोटोज़ देखीं? बहुत शानदार हैं।</p>
<p>{day} को मिलते हैं!</p>""",
            ],
            "newsletter": [
                """<div style="max-width:600px;margin:0 auto;font-family:sans-serif;">
<h1>इस हफ़्ते {topic} में</h1>
<p>{topic} की दुनिया से ताज़ा ख़बरें।</p>
<h2>प्रमुख ख़बरें</h2>
<ul>
<li><strong>बड़ी रिलीज़:</strong> {service} v{v}.0 में {topic} सपोर्ट</li>
<li><strong>इंडस्ट्री न्यूज़:</strong> {candidate} {service} में CTO के रूप में शामिल</li>
<li><strong>ट्यूटोरियल:</strong> {topic} शुरू करें</li>
</ul>
<hr>
<p style="color:#999;font-size:12px;">आपने {service}.com पर सब्सक्राइब किया है।
<a href="#">अनसब्सक्राइब</a></p>
</div>""",
            ],
            "commerce": [
                """<div style="max-width:600px;margin:0 auto;">
<h2>ऑर्डर कन्फ़र्म ✓</h2>
<p>ख़रीदारी के लिए धन्यवाद! ऑर्डर सारांश:</p>
<table style="width:100%;border-collapse:collapse;">
<tr><td style="padding:8px;border-bottom:1px solid #eee;">{topic}</td><td style="text-align:right;">₹3,999</td></tr>
<tr><td style="padding:8px;border-bottom:1px solid #eee;">शिपिंग</td><td style="text-align:right;">मुफ़्त</td></tr>
<tr><td style="padding:8px;font-weight:bold;">कुल</td><td style="text-align:right;font-weight:bold;">₹3,999</td></tr>
</table>
<p>ऑर्डर #{order} · डिलीवरी: {day}</p>
</div>""",
            ],
            "notification": [
                """<div style="font-family:monospace;background:#f6f8fa;padding:16px;border-radius:6px;">
<p><strong>{candidate}</strong> ने <a href="#">{project}#{n}</a> पर कमेंट किया:</p>
<blockquote style="border-left:3px solid #ddd;padding-left:12px;color:#555;">
अच्छा लग रहा है! क्या {topic} खाली होने पर एज केस के लिए टेस्ट जोड़ सकते हैं? बाकी सब LGTM।
</blockquote>
</div>""",
            ],
        },
    },

    "th": {
        "first_names": [
            "สมชาย", "สมหญิง", "วิชัย", "สุภาพร", "ประเสริฐ", "นารี", "ธนากร", "พิมพ์ใจ",
            "อนุชา", "ดวงใจ", "ศักดิ์ชัย", "รัตนา", "กิตติ", "วรรณา", "พงศ์เทพ", "จินดา",
        ],
        "last_names": [
            "สุขใจ", "ใจดี", "รักษา", "พิทักษ์", "บุญมี", "ศรีสุข", "วงศ์สวัสดิ์", "ทองดี",
            "แก้วมณี", "สมบูรณ์", "พันธ์ทอง", "จันทร์เพ็ญ", "สุวรรณ", "เจริญ", "ประสิทธิ์", "มงคล",
        ],
        "romanized_first": [
            "somchai", "somying", "wichai", "supaporn", "prasert", "naree", "thanakorn", "pimjai",
            "anucha", "duangjai", "sakchai", "rattana", "kitti", "wanna", "pongthep", "jinda",
        ],
        "romanized_last": [
            "sukjai", "jaidee", "raksa", "pitak", "boonmee", "srisuk", "wongsawat", "thongdee",
            "kaewmanee", "somboon", "phanthong", "chanpen", "suwan", "charoen", "prasit", "mongkhon",
        ],
        "domains": ["gmail.com", "hotmail.com", "yahoo.co.th", "outlook.com", "naver.com"],
        "days": ["วันจันทร์", "วันอังคาร", "วันพุธ", "วันพฤหัสบดี", "วันศุกร์"],
        "months": ["มกราคม", "กุมภาพันธ์", "มีนาคม", "เมษายน", "พฤษภาคม", "มิถุนายน",
                   "กรกฎาคม", "สิงหาคม", "กันยายน", "ตุลาคม", "พฤศจิกายน", "ธันวาคม"],
        "re_prefix": "Re:",
        "topics": [
            "ไมโครเซอร์วิส", "การย้ายคลาวด์", "รีแฟคเตอร์ฟรอนต์เอนด์", "ตรวจสอบความปลอดภัย",
            "ปรับปรุงประสิทธิภาพ", "ปรับแต่งฐานข้อมูล", "ออกแบบ API", "ทดสอบอัตโนมัติ",
            "ถ่ายภาพ", "ทำอาหาร", "ท่องเที่ยว", "อ่านหนังสือ", "มวยไทย", "ดนตรี",
        ],
        "projects": ["ช้าง", "นาคา", "สยาม", "ครุฑ", "ราชสีห์", "กินรี", "พญานาค", "หงส์"],
        "teams": ["พัฒนา", "ออกแบบ", "ผลิตภัณฑ์", "โครงสร้างพื้นฐาน", "คุณภาพ", "ปฏิบัติการ"],
        "services": ["บริการยืนยันตัวตน", "API Gateway", "PostgreSQL", "Redis", "ระบบมอนิเตอร์", "คิวข้อความ"],
        "subject_templates": {
            "work": [
                "ลำดับความสำคัญ {team} ไตรมาส {q} ปี {year}",
                "Re: บันทึกการทบทวนสปรินต์",
                "อัปเดตไทม์ไลน์ {project}",
                "{project}: เช็คลิสต์การ deploy",
                "รายการดำเนินการจากสแตนด์อัป {day}",
                "RFC: ข้อเสนอรีดีไซน์ {topic}",
                "แจ้งปิดปรับปรุง {service} ({day})",
                "[{team}] วาระประชุมประจำสัปดาห์",
                "รีวิวดีไซน์ {project} v{v}",
                "วาระ 1:1 ({day})",
            ],
            "newsletter": [
                "ข่าว {topic} ประจำสัปดาห์ — ฉบับที่ {n}",
                "สรุป {topic} รายสัปดาห์",
                "สรุป{month}จาก {service}",
                "สรุปสำหรับนักพัฒนา: พิเศษ {topic}",
            ],
            "commerce": [
                "คำสั่งซื้อ #{order} จัดส่งแล้ว!",
                "ยืนยันคำสั่งซื้อ — #{order}",
                "ใบเสร็จจาก {service}",
                "ต่ออายุสมาชิก: {service}",
                "ยืนยันการชำระเงิน — ใบแจ้งหนี้ #{n}",
            ],
            "personal": [
                "Re: ทานข้าว{day}ไหม?",
                "รูปจากทริป!",
                "สุขสันต์วันเกิด! 🎉",
                "Re: แผนสุดสัปดาห์",
                "เจอบทความเรื่อง{topic}มา",
                "แจ้งย้ายบ้าน — ที่อยู่ใหม่",
                "Re: หนังสือที่แนะนำ",
                "ไม่ได้เจอกันนาน! เป็นไงบ้าง?",
            ],
            "notification": [
                "[GitHub] คอมเมนต์ใหม่บน PR #{n}",
                "[GitHub] push ใหม่ใน {project}",
                "[Jira] {project}-{n}: สถานะเปลี่ยนเป็นรีวิว",
                "[Slack] ข้อความใหม่ใน #{team}",
                "[CI] บิลด์ {status} — {project}@main",
                "[Sentry] ปัญหาใหม่ใน {service}",
            ],
        },
        "body_templates": {
            "work": [
                """<p>สวัสดีทุกคน</p>
<p>ติดตามจากการประชุม{day} สถานะปัจจุบัน:</p>
<ul>
<li>การย้าย {project} เสร็จ {pct}% แล้ว</li>
<li>{candidate} ดูแลการเชื่อมต่อ {service}</li>
<li>ต้องสรุปสเปค {topic} ภายในสัปดาห์นี้</li>
</ul>
<p>หากมีปัญหาติดขัดแจ้งได้เลยครับ/ค่ะ</p>
<p>ขอบคุณครับ/ค่ะ<br>{sender}</p>""",
            ],
            "personal": [
                """<p>สวัสดี!</p>
<p>ได้เลย {day} ทานข้าวกัน ร้าน{topic}ใหม่ที่ซอย 5 ดีไหม? ได้ยินว่าอร่อยมาก</p>
<p>เห็นรูปทริปของ{candidate}ไหม? สวยมากเลย</p>
<p>เจอกัน{day}นะ!</p>""",
            ],
            "newsletter": [
                """<div style="max-width:600px;margin:0 auto;font-family:sans-serif;">
<h1>ข่าว {topic} ประจำสัปดาห์</h1>
<p>สรุปข่าวล่าสุดในโลก {topic}</p>
<h2>ข่าวเด่น</h2>
<ul>
<li><strong>เวอร์ชันใหม่:</strong> {service} v{v}.0 รองรับ {topic}</li>
<li><strong>ข่าววงการ:</strong> {candidate} เข้าร่วม {service} ในตำแหน่ง CTO</li>
<li><strong>บทเรียน:</strong> เริ่มต้นกับ {topic}</li>
</ul>
<hr>
<p style="color:#999;font-size:12px;">คุณสมัครรับข่าวจาก {service}.com
<a href="#">ยกเลิกการรับข่าว</a></p>
</div>""",
            ],
            "commerce": [
                """<div style="max-width:600px;margin:0 auto;">
<h2>ยืนยันคำสั่งซื้อ ✓</h2>
<p>ขอบคุณที่สั่งซื้อ! รายละเอียด:</p>
<table style="width:100%;border-collapse:collapse;">
<tr><td style="padding:8px;border-bottom:1px solid #eee;">{topic}</td><td style="text-align:right;">฿1,490</td></tr>
<tr><td style="padding:8px;border-bottom:1px solid #eee;">ค่าจัดส่ง</td><td style="text-align:right;">ฟรี</td></tr>
<tr><td style="padding:8px;font-weight:bold;">รวม</td><td style="text-align:right;font-weight:bold;">฿1,490</td></tr>
</table>
<p>เลขที่คำสั่งซื้อ #{order} · จัดส่งภายใน: {day}</p>
</div>""",
            ],
            "notification": [
                """<div style="font-family:monospace;background:#f6f8fa;padding:16px;border-radius:6px;">
<p><strong>{candidate}</strong> แสดงความเห็นบน <a href="#">{project}#{n}</a>:</p>
<blockquote style="border-left:3px solid #ddd;padding-left:12px;color:#555;">
ดูดีครับ! เพิ่มเทสต์สำหรับ edge case เมื่อ {topic} ว่างเปล่าได้ไหม? นอกนั้น LGTM ครับ
</blockquote>
</div>""",
            ],
        },
    },
}

# Available non-Latin locale keys
I18N_KEYS = list(I18N_LOCALES.keys())  # ["ja", "zh", "ko", "ar", "ru", "hi", "th"]

ACCOUNT_PRESETS = [
    {"email": "alex.morgan@gmail.com",    "name": "Alex Morgan",    "provider": "gmail_api", "host": "imap.gmail.com"},
    {"email": "alex.morgan@company.io",   "name": "Alex Morgan",    "provider": "imap",      "host": "mail.company.io"},
    {"email": "a.morgan@outlook.com",     "name": "Alex Morgan",    "provider": "graph",     "host": "outlook.office365.com"},
    {"email": "alex@fastmail.com",        "name": "Alex Morgan",    "provider": "jmap",      "host": "jmap.fastmail.com"},
]

SYSTEM_LABELS = [
    {"name": "INBOX",     "special": "inbox",   "sort": 0},
    {"name": "Sent",      "special": "sent",    "sort": 1},
    {"name": "Drafts",    "special": "drafts",  "sort": 2},
    {"name": "Trash",     "special": "trash",   "sort": 3},
    {"name": "Archive",   "special": "archive", "sort": 4},
    {"name": "Spam",      "special": "junk",    "sort": 5},
]

USER_LABELS = [
    {"name": "Work",         "color_bg": "#4285f4", "color_fg": "#ffffff"},
    {"name": "Personal",     "color_bg": "#0b8043", "color_fg": "#ffffff"},
    {"name": "Finance",      "color_bg": "#f4b400", "color_fg": "#000000"},
    {"name": "Travel",       "color_bg": "#db4437", "color_fg": "#ffffff"},
    {"name": "Newsletters",  "color_bg": "#ab47bc", "color_fg": "#ffffff"},
    {"name": "Receipts",     "color_bg": "#00acc1", "color_fg": "#ffffff"},
    {"name": "Projects",     "color_bg": "#ff7043", "color_fg": "#ffffff"},
    {"name": "Waiting",      "color_bg": "#8d6e63", "color_fg": "#ffffff"},
]

# ── Helpers ──────────────────────────────────────────────────

rng = random.Random(42)  # deterministic for reproducibility

def gen_id():
    return str(uuid.UUID(int=rng.getrandbits(128)))

def gen_person(locale=None):
    """Generate a (display_name, email) pair. locale=None means Latin."""
    if locale and locale in I18N_LOCALES:
        loc = I18N_LOCALES[locale]
        idx = rng.randrange(len(loc["first_names"]))
        first_native = loc["first_names"][idx]
        last_idx = rng.randrange(len(loc["last_names"]))
        last_native = loc["last_names"][last_idx]
        # Email uses romanized form
        rom_first = loc["romanized_first"][idx]
        rom_last = loc["romanized_last"][last_idx]
        domain = rng.choice(loc["domains"])
        email = f"{rom_first}.{rom_last}@{domain}"
        return f"{first_native} {last_native}", email
    else:
        first = rng.choice(FIRST_NAMES)
        last = rng.choice(LAST_NAMES)
        domain = rng.choice(DOMAINS)
        email = f"{first.lower()}.{last.lower().replace(' ', '')}@{domain}"
        return f"{first} {last}", email

def gen_message_id(domain="mail.ratatoskr.test"):
    return f"<{uuid.UUID(int=rng.getrandbits(128)).hex[:16]}@{domain}>"

def fill_template(template):
    """Replace placeholders in a template string."""
    return template.format(
        q=rng.randint(1, 4),
        year=rng.choice([2024, 2025, 2026]),
        team=rng.choice(TEAMS),
        project=rng.choice(PROJECTS),
        day=rng.choice(DAYS),
        topic=rng.choice(TOPICS),
        service=rng.choice(SERVICES),
        candidate=rng.choice(FIRST_NAMES) + " " + rng.choice(LAST_NAMES),
        v=rng.randint(1, 9),
        month=rng.choice(MONTHS),
        n=rng.randint(100, 9999),
        order=uuid.UUID(int=rng.getrandbits(128)).hex[:8].upper(),
        pct=rng.choice([15, 25, 40, 60, 75, 80, 90, 95]),
        status=rng.choice(STATUSES),
        sender=rng.choice(FIRST_NAMES),
        recipient=rng.choice(FIRST_NAMES),
    )

def fill_template_i18n(template, locale):
    """Replace placeholders using locale-specific data."""
    loc = I18N_LOCALES[locale]
    return template.format(
        q=rng.randint(1, 4),
        year=rng.choice([2024, 2025, 2026]),
        team=rng.choice(loc["teams"]),
        project=rng.choice(loc["projects"]),
        day=rng.choice(loc["days"]),
        topic=rng.choice(loc["topics"]),
        service=rng.choice(loc["services"]),
        candidate=rng.choice(loc["first_names"]) + " " + rng.choice(loc["last_names"]),
        v=rng.randint(1, 9),
        month=rng.choice(loc["months"]),
        n=rng.randint(100, 9999),
        order=uuid.UUID(int=rng.getrandbits(128)).hex[:8].upper(),
        pct=rng.choice([15, 25, 40, 60, 75, 80, 90, 95]),
        status=rng.choice(STATUSES),
        sender=rng.choice(loc["first_names"]),
        recipient=rng.choice(loc["first_names"]),
    )

def random_date(days_back=365):
    """Random datetime within the last N days."""
    now = datetime.now(timezone.utc)
    delta = timedelta(
        days=rng.randint(0, days_back),
        hours=rng.randint(0, 23),
        minutes=rng.randint(0, 59),
        seconds=rng.randint(0, 59),
    )
    return now - delta

def ts(dt):
    """datetime to unix timestamp."""
    return int(dt.timestamp())

# ── Create main database ─────────────────────────────────────

db = sqlite3.connect(str(OUT_DB))
db.execute("PRAGMA journal_mode = WAL")
db.execute("PRAGMA foreign_keys = ON")

# Schema matching the seed-db.py but with extra columns from migrations
db.executescript("""
    CREATE TABLE _migrations (
        version INTEGER PRIMARY KEY,
        description TEXT,
        applied_at INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE accounts (
        id TEXT PRIMARY KEY,
        email TEXT NOT NULL UNIQUE,
        display_name TEXT,
        avatar_url TEXT,
        access_token TEXT,
        refresh_token TEXT,
        token_expires_at INTEGER,
        history_id TEXT,
        last_sync_at INTEGER,
        is_active INTEGER DEFAULT 1,
        created_at INTEGER DEFAULT (unixepoch()),
        updated_at INTEGER DEFAULT (unixepoch()),
        provider TEXT DEFAULT 'imap',
        imap_host TEXT,
        imap_port INTEGER DEFAULT 993,
        imap_security TEXT DEFAULT 'tls',
        smtp_host TEXT,
        smtp_port INTEGER DEFAULT 587,
        smtp_security TEXT DEFAULT 'starttls',
        auth_method TEXT DEFAULT 'oauth2',
        imap_password TEXT,
        oauth_provider TEXT,
        oauth_client_id TEXT,
        oauth_client_secret TEXT,
        imap_username TEXT,
        caldav_url TEXT,
        caldav_username TEXT,
        caldav_password TEXT,
        caldav_principal_url TEXT,
        caldav_home_url TEXT,
        calendar_provider TEXT,
        accept_invalid_certs INTEGER DEFAULT 0,
        jmap_url TEXT,
        account_color TEXT,
        account_name TEXT,
        sort_order INTEGER DEFAULT 0,
        initial_sync_completed INTEGER DEFAULT 1,
        smtp_username TEXT,
        smtp_password TEXT,
        oauth_token_url TEXT
    );

    CREATE TABLE labels (
        id TEXT NOT NULL,
        account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        type TEXT NOT NULL,
        color_bg TEXT,
        color_fg TEXT,
        visible INTEGER DEFAULT 1,
        sort_order INTEGER DEFAULT 0,
        imap_folder_path TEXT,
        imap_special_use TEXT,
        label_kind TEXT DEFAULT 'tag',
        parent_label_id TEXT,
        namespace_type TEXT,
        right_read INTEGER DEFAULT 1,
        right_add INTEGER DEFAULT 1,
        right_remove INTEGER DEFAULT 1,
        right_set_seen INTEGER DEFAULT 1,
        right_set_keywords INTEGER DEFAULT 1,
        right_create_child INTEGER DEFAULT 0,
        right_rename INTEGER DEFAULT 0,
        right_delete INTEGER DEFAULT 0,
        right_submit INTEGER DEFAULT 0,
        is_subscribed INTEGER DEFAULT 1,
        PRIMARY KEY (account_id, id)
    );
    CREATE INDEX idx_labels_account ON labels(account_id);

    CREATE TABLE threads (
        id TEXT NOT NULL,
        account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        subject TEXT,
        snippet TEXT,
        last_message_at INTEGER,
        message_count INTEGER DEFAULT 0,
        is_read INTEGER DEFAULT 0,
        is_starred INTEGER DEFAULT 0,
        is_important INTEGER DEFAULT 0,
        has_attachments INTEGER DEFAULT 0,
        is_snoozed INTEGER DEFAULT 0,
        snooze_until INTEGER,
        is_pinned INTEGER DEFAULT 0,
        is_muted INTEGER DEFAULT 0,
        PRIMARY KEY (account_id, id)
    );
    CREATE INDEX idx_threads_date ON threads(account_id, last_message_at DESC);
    CREATE INDEX idx_threads_snoozed ON threads(is_snoozed) WHERE is_snoozed = 1;
    CREATE INDEX idx_threads_pinned ON threads(is_pinned) WHERE is_pinned = 1;
    CREATE INDEX idx_threads_muted ON threads(is_muted) WHERE is_muted = 1;

    CREATE TABLE thread_labels (
        thread_id TEXT NOT NULL,
        account_id TEXT NOT NULL,
        label_id TEXT NOT NULL,
        PRIMARY KEY (account_id, thread_id, label_id),
        FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
    );
    CREATE INDEX idx_thread_labels_label ON thread_labels(account_id, label_id);

    CREATE TABLE messages (
        id TEXT NOT NULL,
        account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        thread_id TEXT NOT NULL,
        from_address TEXT,
        from_name TEXT,
        to_addresses TEXT,
        cc_addresses TEXT,
        bcc_addresses TEXT,
        reply_to TEXT,
        subject TEXT,
        snippet TEXT,
        date INTEGER NOT NULL,
        is_read INTEGER DEFAULT 0,
        is_starred INTEGER DEFAULT 0,
        body_cached INTEGER DEFAULT 0,
        raw_size INTEGER,
        internal_date INTEGER,
        list_unsubscribe TEXT,
        list_unsubscribe_post TEXT,
        auth_results TEXT,
        message_id_header TEXT,
        references_header TEXT,
        in_reply_to_header TEXT,
        imap_uid INTEGER,
        imap_folder TEXT,
        is_mentioned INTEGER DEFAULT 0,
        is_reaction INTEGER DEFAULT 0,
        mdn_requested INTEGER DEFAULT 0,
        mdn_sent INTEGER DEFAULT 0,
        PRIMARY KEY (account_id, id),
        FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE
    );
    CREATE INDEX idx_messages_thread ON messages(account_id, thread_id, date ASC);
    CREATE INDEX idx_messages_date ON messages(account_id, date DESC);
    CREATE INDEX idx_messages_from ON messages(from_address);
    CREATE INDEX idx_messages_imap_uid ON messages(account_id, imap_folder, imap_uid);
    CREATE INDEX idx_messages_message_id ON messages(message_id_header);
    CREATE INDEX idx_messages_is_mentioned ON messages(is_mentioned) WHERE is_mentioned = 1;

    CREATE TABLE attachments (
        id TEXT PRIMARY KEY,
        message_id TEXT NOT NULL,
        account_id TEXT NOT NULL,
        filename TEXT,
        mime_type TEXT,
        size INTEGER,
        gmail_attachment_id TEXT,
        content_id TEXT,
        is_inline INTEGER DEFAULT 0,
        local_path TEXT,
        imap_part_id TEXT,
        content_hash TEXT,
        cached_at INTEGER,
        cache_size INTEGER
    );
    CREATE INDEX idx_attachments_message ON attachments(account_id, message_id);
    CREATE INDEX idx_attachments_cid ON attachments(content_id) WHERE content_id IS NOT NULL;
    CREATE INDEX idx_attachments_content_hash ON attachments(content_hash) WHERE content_hash IS NOT NULL;

    CREATE TABLE contacts (
        id TEXT PRIMARY KEY,
        email TEXT NOT NULL UNIQUE,
        display_name TEXT,
        avatar_url TEXT,
        frequency INTEGER DEFAULT 1,
        last_contacted_at INTEGER,
        notes TEXT,
        created_at INTEGER DEFAULT (unixepoch()),
        updated_at INTEGER DEFAULT (unixepoch()),
        email2 TEXT,
        phone TEXT,
        company TEXT,
        source TEXT DEFAULT 'local',
        display_name_overridden INTEGER DEFAULT 0,
        server_id TEXT
    );

    CREATE TABLE seen_addresses (
        email TEXT PRIMARY KEY,
        display_name TEXT,
        send_count INTEGER DEFAULT 0,
        receive_count INTEGER DEFAULT 0,
        last_seen_at INTEGER,
        first_seen_at INTEGER
    );

    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE thread_ui_state (
        account_id TEXT NOT NULL,
        thread_id TEXT NOT NULL,
        attachments_collapsed INTEGER DEFAULT 1,
        PRIMARY KEY (account_id, thread_id)
    );

    CREATE TABLE local_drafts (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL,
        thread_id TEXT,
        in_reply_to_message_id TEXT,
        from_address TEXT,
        to_addresses TEXT,
        cc_addresses TEXT,
        bcc_addresses TEXT,
        subject TEXT,
        body_html TEXT,
        body_text TEXT,
        created_at INTEGER DEFAULT (unixepoch()),
        updated_at INTEGER DEFAULT (unixepoch()),
        sync_status TEXT DEFAULT 'local'
    );

    CREATE TABLE smart_folders (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        query TEXT NOT NULL,
        icon TEXT,
        sort_order INTEGER DEFAULT 0,
        created_at INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE filter_rules (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        criteria_json TEXT NOT NULL,
        actions_json TEXT NOT NULL,
        is_active INTEGER DEFAULT 1,
        sort_order INTEGER DEFAULT 0,
        created_at INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE notification_vips (
        email TEXT PRIMARY KEY,
        display_name TEXT,
        created_at INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE image_allowlist (
        sender_address TEXT PRIMARY KEY,
        created_at INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE follow_up_reminders (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL,
        thread_id TEXT NOT NULL,
        message_id TEXT,
        remind_at INTEGER NOT NULL,
        status TEXT DEFAULT 'pending',
        created_at INTEGER DEFAULT (unixepoch())
    );
    CREATE INDEX idx_follow_up_status ON follow_up_reminders(status, remind_at);

    CREATE TABLE signatures (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL,
        name TEXT NOT NULL,
        body_html TEXT,
        body_text TEXT,
        is_default INTEGER DEFAULT 0,
        is_reply_default INTEGER DEFAULT 0,
        sort_order INTEGER DEFAULT 0,
        source TEXT DEFAULT 'local',
        server_id TEXT,
        created_at INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE categories (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        color TEXT,
        account_id TEXT,
        source TEXT DEFAULT 'local'
    );

    CREATE TABLE label_color_overrides (
        account_id TEXT NOT NULL,
        label_id TEXT NOT NULL,
        color_bg TEXT,
        color_fg TEXT,
        PRIMARY KEY (account_id, label_id)
    );

    CREATE TABLE ai_cache (
        account_id TEXT NOT NULL,
        thread_id TEXT NOT NULL,
        type TEXT NOT NULL,
        value TEXT,
        created_at INTEGER DEFAULT (unixepoch()),
        PRIMARY KEY (account_id, thread_id, type)
    );

    CREATE TABLE scheduled_emails (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL,
        thread_id TEXT,
        send_at INTEGER NOT NULL,
        from_address TEXT,
        to_addresses TEXT,
        cc_addresses TEXT,
        bcc_addresses TEXT,
        subject TEXT,
        body_html TEXT,
        body_text TEXT,
        status TEXT DEFAULT 'scheduled',
        created_at INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE pending_operations (
        id TEXT PRIMARY KEY,
        account_id TEXT NOT NULL,
        operation_type TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        status TEXT DEFAULT 'pending',
        retry_count INTEGER DEFAULT 0,
        created_at INTEGER DEFAULT (unixepoch()),
        last_attempted_at INTEGER
    );

    CREATE TABLE folder_sync_state (
        account_id TEXT NOT NULL,
        folder_path TEXT NOT NULL,
        uidvalidity INTEGER,
        last_uid INTEGER,
        modseq INTEGER,
        last_sync_at INTEGER,
        last_deletion_check_at INTEGER,
        PRIMARY KEY (account_id, folder_path)
    );

    CREATE TABLE bundle_rules (
        id TEXT PRIMARY KEY,
        category TEXT NOT NULL,
        max_hold_minutes INTEGER DEFAULT 60,
        is_active INTEGER DEFAULT 1,
        created_at INTEGER DEFAULT (unixepoch())
    );

    CREATE TABLE bundled_threads (
        account_id TEXT NOT NULL,
        thread_id TEXT NOT NULL,
        bundle_rule_id TEXT NOT NULL,
        held_until INTEGER NOT NULL,
        PRIMARY KEY (account_id, thread_id)
    );
""")

# ── Insert accounts ──────────────────────────────────────────

ACCOUNT_COLORS = ["#4285f4", "#ea4335", "#fbbc04", "#34a853"]
accounts = []

for i in range(NUM_ACCOUNTS):
    preset = ACCOUNT_PRESETS[i]
    acc_id = gen_id()
    acc = {
        "id": acc_id,
        "email": preset["email"],
        "name": preset["name"],
        "provider": preset["provider"],
        "host": preset["host"],
        "labels": {},  # label_id -> label_info
        "inbox_label_id": None,
        "sent_label_id": None,
    }
    db.execute("""
        INSERT INTO accounts (id, email, display_name, provider, imap_host, auth_method,
                              account_color, account_name, sort_order, initial_sync_completed)
        VALUES (?, ?, ?, ?, ?, 'oauth2', ?, ?, ?, 1)
    """, (acc_id, preset["email"], preset["name"], preset["provider"],
          preset["host"], ACCOUNT_COLORS[i], preset["name"], i))
    accounts.append(acc)

print(f"  Accounts: {len(accounts)}")

# ── Insert labels ────────────────────────────────────────────

for acc in accounts:
    # System labels
    for sl in SYSTEM_LABELS:
        lid = gen_id()
        db.execute("""
            INSERT INTO labels (id, account_id, name, type, imap_folder_path,
                                imap_special_use, sort_order, label_kind)
            VALUES (?, ?, ?, 'system', ?, ?, ?, 'container')
        """, (lid, acc["id"], sl["name"], sl["name"], sl["special"], sl["sort"]))
        acc["labels"][lid] = sl
        if sl["special"] == "inbox":
            acc["inbox_label_id"] = lid
        elif sl["special"] == "sent":
            acc["sent_label_id"] = lid

    # User labels
    for ul in USER_LABELS:
        lid = gen_id()
        db.execute("""
            INSERT INTO labels (id, account_id, name, type, color_bg, color_fg,
                                sort_order, label_kind)
            VALUES (?, ?, ?, 'user', ?, ?, ?, 'tag')
        """, (lid, acc["id"], ul["name"], ul["color_bg"], ul["color_fg"], 10 + USER_LABELS.index(ul)))
        acc["labels"][lid] = ul

label_count = db.execute("SELECT count(*) FROM labels").fetchone()[0]
print(f"  Labels:   {label_count}")

# ── Insert smart folders ─────────────────────────────────────

smart_folders = [
    ("Unread",       "is:unread",                          "📬", 0),
    ("Attachments",  "has:attachment",                      "📎", 1),
    ("Starred",      "is:starred",                          "⭐", 2),
    ("Recent",       "is:unread after:__LAST_7_DAYS__",     "🕐", 3),
]
for name, query, icon, sort in smart_folders:
    db.execute("INSERT INTO smart_folders (id, name, query, icon, sort_order) VALUES (?, ?, ?, ?, ?)",
               (gen_id(), name, query, icon, sort))

# ── Insert signatures ────────────────────────────────────────

for acc in accounts:
    db.execute("""
        INSERT INTO signatures (id, account_id, name, body_html, body_text, is_default)
        VALUES (?, ?, 'Default', ?, ?, 1)
    """, (gen_id(), acc["id"],
          f'<p>Best regards,<br><strong>{acc["name"]}</strong><br>{acc["email"]}</p>',
          f'Best regards,\n{acc["name"]}\n{acc["email"]}'))

# ── Insert settings ──────────────────────────────────────────

default_settings = {
    "theme": "system",
    "sync_period_days": "90",
    "notifications_enabled": "true",
    "compact_list": "false",
    "conversation_view": "true",
    "signature_on_reply": "true",
}
for k, v in default_settings.items():
    db.execute("INSERT INTO settings (key, value) VALUES (?, ?)", (k, v))

# ── Generate people pools ────────────────────────────────────

# Latin pool
latin_people_pool = []
seen_emails = set()
for _ in range(200):
    name, email = gen_person(locale=None)
    if email not in seen_emails:
        seen_emails.add(email)
        latin_people_pool.append((name, email))

# Per-locale pools
i18n_people_pools = {}
for loc_key in I18N_KEYS:
    pool = []
    for _ in range(60):
        name, email = gen_person(locale=loc_key)
        if email not in seen_emails:
            seen_emails.add(email)
            pool.append((name, email))
    i18n_people_pools[loc_key] = pool

# Combined pool for VIPs / contacts
people_pool = latin_people_pool[:]
for pool in i18n_people_pools.values():
    people_pool.extend(pool)

# ── Create body store ────────────────────────────────────────

body_db = sqlite3.connect(str(BODIES_DB))
body_db.execute("PRAGMA journal_mode = WAL")
body_db.executescript("""
    CREATE TABLE IF NOT EXISTS bodies (
        message_id TEXT PRIMARY KEY,
        body_html  BLOB,
        body_text  BLOB
    );
""")

# ── Try to import zstandard for compression ──────────────────

try:
    import zstandard as zstd
    compressor = zstd.ZstdCompressor(level=3)
    def compress(data: bytes) -> bytes:
        return compressor.compress(data)
    print("  Using zstd compression for body store")
except ImportError:
    # Fallback: store uncompressed (the app can still read raw data)
    def compress(data: bytes) -> bytes:
        return data
    print("  WARNING: zstandard not installed — bodies stored uncompressed")
    print("  Install with: pip install zstandard")

# ── Generate threads and messages ────────────────────────────

ATTACHMENT_NAMES = [
    ("report.pdf", "application/pdf", 245_000),
    ("screenshot.png", "image/png", 890_000),
    ("proposal.docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", 156_000),
    ("data.xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", 78_000),
    ("design-v3.fig", "application/octet-stream", 2_400_000),
    ("meeting-notes.md", "text/markdown", 4_200),
    ("invoice-2024.pdf", "application/pdf", 67_000),
    ("photo.jpg", "image/jpeg", 3_100_000),
    ("logo.svg", "image/svg+xml", 12_000),
    ("archive.zip", "application/zip", 15_600_000),
    ("presentation.pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation", 4_500_000),
    ("contract-final.pdf", "application/pdf", 189_000),
    ("wireframes.pdf", "application/pdf", 1_200_000),
    ("budget.csv", "text/csv", 23_000),
]

thread_count = 0
message_count = 0
attachment_count = 0
imap_uid_counter = {}  # per (account_id, folder)

# Category weights for thread generation
CATEGORIES = ["work", "newsletter", "commerce", "personal", "notification"]
CATEGORY_WEIGHTS = [0.35, 0.15, 0.10, 0.20, 0.20]

for _ in range(NUM_THREADS):
    acc = rng.choice(accounts)
    category = rng.choices(CATEGORIES, weights=CATEGORY_WEIGHTS, k=1)[0]

    # Pick locale for this thread
    if LOCALE_MODE == "latin":
        thread_locale = None
    elif LOCALE_MODE == "intl":
        thread_locale = rng.choice(I18N_KEYS)
    else:  # mixed — ~30% non-Latin
        thread_locale = rng.choice(I18N_KEYS) if rng.random() < 0.30 else None

    # Generate subject
    if thread_locale:
        loc = I18N_LOCALES[thread_locale]
        template = rng.choice(loc["subject_templates"][category])
        subject = fill_template_i18n(template, thread_locale)
        re_prefix = loc["re_prefix"]
        thread_people = i18n_people_pools[thread_locale]
    else:
        template = rng.choice(SUBJECT_TEMPLATES[category])
        subject = fill_template(template)
        re_prefix = "Re:"
        thread_people = latin_people_pool

    # Thread timing
    thread_start = random_date(days_back=365)

    # Number of messages in thread (weighted toward fewer)
    if category == "newsletter":
        num_msgs = 1
    elif category == "commerce":
        num_msgs = rng.choices([1, 2, 3], weights=[0.7, 0.2, 0.1], k=1)[0]
    elif category == "notification":
        num_msgs = 1
    elif category == "work":
        num_msgs = rng.choices([1, 2, 3, 4, 5, 6, 7, 8, 9, 12], weights=[0.10, 0.12, 0.12, 0.10, 0.13, 0.10, 0.10, 0.10, 0.08, 0.05], k=1)[0]
    else:
        num_msgs = rng.choices([1, 2, 3, 4, 5, 8, 12], weights=[0.30, 0.25, 0.15, 0.10, 0.10, 0.05, 0.05], k=1)[0]

    # Participants
    num_participants = min(rng.choices([1, 2, 3, 4, 5], weights=[0.1, 0.4, 0.25, 0.15, 0.1], k=1)[0], len(thread_people))
    participants = rng.sample(thread_people, num_participants)

    # Own email is in to: for received, from: for sent
    own_email = acc["email"]
    own_name = acc["name"]

    thread_id = gen_id()
    is_read = rng.random() < 0.7
    is_starred = rng.random() < 0.08
    is_pinned = rng.random() < 0.02
    is_snoozed = rng.random() < 0.03
    snooze_until = ts(datetime.now(timezone.utc) + timedelta(days=rng.randint(1, 14))) if is_snoozed else None
    has_attachments = False
    is_important = rng.random() < 0.05

    # Determine folder: most go to inbox, some to sent, archive, trash
    folder_weights = {"INBOX": 0.70, "Sent": 0.10, "Archive": 0.12, "Trash": 0.03, "Spam": 0.02, "Drafts": 0.03}
    folder_name = rng.choices(list(folder_weights.keys()), weights=list(folder_weights.values()), k=1)[0]

    # Build messages
    thread_messages = []
    msg_refs = []
    first_msg_id_header = None

    for mi in range(num_msgs):
        msg_id = gen_id()
        msg_id_header = gen_message_id()

        if mi == 0:
            first_msg_id_header = msg_id_header
            in_reply_to = None
            references = None
        else:
            in_reply_to = msg_refs[-1]
            references = " ".join(msg_refs)

        msg_refs.append(msg_id_header)

        # Alternate sender between participants and self
        if num_msgs == 1:
            # Single message: usually received
            sender_name, sender_email = participants[0]
            to_addr = f"{own_name} <{own_email}>"
        elif mi % 2 == 0:
            # Even messages: from others
            sender_name, sender_email = participants[mi % len(participants)]
            to_addr = f"{own_name} <{own_email}>"
        else:
            # Odd messages: from self (replies)
            sender_name, sender_email = own_name, own_email
            to_addr = ", ".join(f"{n} <{e}>" for n, e in participants[:3])

        # CC sometimes
        cc = None
        if num_participants > 2 and rng.random() < 0.3:
            cc_people = participants[2:4]
            cc = ", ".join(f"{n} <{e}>" for n, e in cc_people)

        # Message date: spread across thread duration
        msg_date = thread_start + timedelta(
            hours=mi * rng.randint(1, 48),
            minutes=rng.randint(0, 59),
        )

        # Snippet
        snippet_text = subject[:200] if mi == 0 else f"{re_prefix} {subject}"[:200]

        # Is this message read?
        msg_is_read = True if mi < num_msgs - 1 else is_read

        # Attachments (on ~20% of work/personal messages)
        msg_attachments = []
        if rng.random() < 0.20 and category in ("work", "personal", "commerce"):
            num_att = rng.choices([1, 2, 3], weights=[0.6, 0.3, 0.1], k=1)[0]
            for _ in range(num_att):
                att_info = rng.choice(ATTACHMENT_NAMES)
                msg_attachments.append({
                    "id": gen_id(),
                    "filename": att_info[0],
                    "mime_type": att_info[1],
                    "size": att_info[2] + rng.randint(-att_info[2]//4, att_info[2]//4),
                })
            has_attachments = True

        # IMAP UID
        folder_key = (acc["id"], folder_name)
        imap_uid_counter.setdefault(folder_key, 0)
        imap_uid_counter[folder_key] += 1
        imap_uid = imap_uid_counter[folder_key]

        # List-Unsubscribe for newsletters
        list_unsub = None
        list_unsub_post = None
        if category == "newsletter":
            list_unsub = f"<https://newsletter.example.com/unsubscribe/{uuid.UUID(int=rng.getrandbits(128)).hex[:12]}>"
            list_unsub_post = "List-Unsubscribe=One-Click"

        thread_messages.append({
            "id": msg_id,
            "account_id": acc["id"],
            "thread_id": thread_id,
            "from_name": sender_name,
            "from_address": sender_email,
            "to_addresses": to_addr,
            "cc_addresses": cc,
            "subject": subject if mi == 0 else f"{re_prefix} {subject}",
            "snippet": snippet_text,
            "date": ts(msg_date),
            "is_read": int(msg_is_read),
            "is_starred": int(is_starred and mi == 0),
            "message_id_header": msg_id_header,
            "references_header": references,
            "in_reply_to_header": in_reply_to,
            "imap_uid": imap_uid,
            "imap_folder": folder_name,
            "list_unsubscribe": list_unsub,
            "list_unsubscribe_post": list_unsub_post,
            "attachments": msg_attachments,
            "raw_size": rng.randint(2000, 50000),
            "msg_date": msg_date,
            "sender_name": sender_name,
            "sender_email": sender_email,
            "category": category,
            "locale": thread_locale,
        })

    # Latest message date
    latest_date = max(m["date"] for m in thread_messages)
    latest_msg = max(thread_messages, key=lambda m: m["date"])

    # Insert thread
    db.execute("""
        INSERT INTO threads (id, account_id, subject, snippet, last_message_at,
                             message_count, is_read, is_starred, has_attachments,
                             is_important, is_pinned, is_snoozed, snooze_until, is_muted)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """, (thread_id, acc["id"], subject, latest_msg["snippet"], latest_date,
          len(thread_messages), int(is_read), int(is_starred), int(has_attachments),
          int(is_important), int(is_pinned), int(is_snoozed), snooze_until,
          int(rng.random() < 0.01)))
    thread_count += 1

    # Thread labels: folder label + maybe user labels
    # Find the inbox/sent/etc label
    for lid, linfo in acc["labels"].items():
        if isinstance(linfo, dict) and linfo.get("name") == folder_name:
            db.execute("INSERT OR IGNORE INTO thread_labels (thread_id, account_id, label_id) VALUES (?, ?, ?)",
                       (thread_id, acc["id"], lid))
            break

    # Add user label based on category
    category_label_map = {
        "work": "Work",
        "personal": "Personal",
        "newsletter": "Newsletters",
        "commerce": "Receipts",
    }
    if category in category_label_map and rng.random() < 0.6:
        target_label = category_label_map[category]
        for lid, linfo in acc["labels"].items():
            if isinstance(linfo, dict) and linfo.get("name") == target_label:
                db.execute("INSERT OR IGNORE INTO thread_labels (thread_id, account_id, label_id) VALUES (?, ?, ?)",
                           (thread_id, acc["id"], lid))
                break

    # Insert messages
    for msg in thread_messages:
        db.execute("""
            INSERT INTO messages (id, account_id, thread_id, from_address, from_name,
                                  to_addresses, cc_addresses, subject, snippet, date,
                                  is_read, is_starred, body_cached, raw_size,
                                  internal_date, message_id_header, references_header,
                                  in_reply_to_header, imap_uid, imap_folder,
                                  list_unsubscribe, list_unsubscribe_post)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """, (msg["id"], msg["account_id"], msg["thread_id"],
              msg["from_address"], msg["from_name"],
              msg["to_addresses"], msg["cc_addresses"],
              msg["subject"], msg["snippet"], msg["date"],
              msg["is_read"], msg["is_starred"], msg["raw_size"],
              msg["date"], msg["message_id_header"], msg["references_header"],
              msg["in_reply_to_header"], msg["imap_uid"], msg["imap_folder"],
              msg["list_unsubscribe"], msg["list_unsubscribe_post"]))
        message_count += 1

        # Generate body HTML and store in body store
        cat = msg["category"]
        msg_locale = msg["locale"]
        if msg_locale and msg_locale in I18N_LOCALES:
            loc_data = I18N_LOCALES[msg_locale]
            if cat in loc_data["body_templates"]:
                body_html = fill_template_i18n(rng.choice(loc_data["body_templates"][cat]), msg_locale)
            else:
                body_html = f"<p>{msg['snippet']}</p>"
        elif cat in BODY_TEMPLATES:
            body_html = fill_template(rng.choice(BODY_TEMPLATES[cat]))
        else:
            body_html = f"<p>{msg['snippet']}</p>"

        # Strip tags for text version (simple approach)
        body_text = re.sub(r'<[^>]+>', '', body_html)
        body_text = re.sub(r'\s+', ' ', body_text).strip()

        body_db.execute(
            "INSERT INTO bodies (message_id, body_html, body_text) VALUES (?, ?, ?)",
            (msg["id"], compress(body_html.encode("utf-8")), compress(body_text.encode("utf-8")))
        )

        # Insert attachments
        for att in msg["attachments"]:
            db.execute("""
                INSERT INTO attachments (id, message_id, account_id, filename, mime_type, size)
                VALUES (?, ?, ?, ?, ?, ?)
            """, (att["id"], msg["id"], msg["account_id"], att["filename"], att["mime_type"], att["size"]))
            attachment_count += 1

        # Upsert contact + seen_address
        if msg["from_address"]:
            db.execute("""
                INSERT INTO contacts (id, email, display_name, frequency, last_contacted_at)
                VALUES (?, ?, ?, 1, ?)
                ON CONFLICT(email) DO UPDATE SET
                    frequency = frequency + 1,
                    display_name = COALESCE(excluded.display_name, display_name),
                    last_contacted_at = MAX(COALESCE(excluded.last_contacted_at, 0),
                                            COALESCE(last_contacted_at, 0))
            """, (gen_id(), msg["from_address"], msg["from_name"], msg["date"]))

            db.execute("""
                INSERT INTO seen_addresses (email, display_name, receive_count, last_seen_at, first_seen_at)
                VALUES (?, ?, 1, ?, ?)
                ON CONFLICT(email) DO UPDATE SET
                    receive_count = receive_count + 1,
                    display_name = COALESCE(excluded.display_name, display_name),
                    last_seen_at = MAX(excluded.last_seen_at, last_seen_at)
            """, (msg["from_address"], msg["from_name"], msg["date"], msg["date"]))

# ── Insert some VIP senders ──────────────────────────────────

vip_people = rng.sample(people_pool, min(5, len(people_pool)))
for name, email in vip_people:
    db.execute("INSERT OR IGNORE INTO notification_vips (email, display_name) VALUES (?, ?)",
               (email, name))

# ── Commit everything ────────────────────────────────────────

db.commit()
body_db.commit()

contact_count = db.execute("SELECT count(*) FROM contacts").fetchone()[0]
seen_count = db.execute("SELECT count(*) FROM seen_addresses").fetchone()[0]

db.close()
body_db.close()

# ── Summary ──────────────────────────────────────────────────

print(f"\nDone!")
print(f"  Accounts:    {len(accounts)}")
print(f"  Labels:      {label_count}")
print(f"  Threads:     {thread_count}")
print(f"  Messages:    {message_count}")
print(f"  Attachments: {attachment_count}")
print(f"  Contacts:    {contact_count}")
print(f"  Seen addrs:  {seen_count}")
print(f"  Smart fldrs: {len(smart_folders)}")
print(f"  VIPs:        {len(vip_people)}")
print(f"\n  Database:    {OUT_DB}")
print(f"  Body store:  {BODIES_DB}")
print(f"  DB size:     {OUT_DB.stat().st_size / 1024 / 1024:.1f} MB")
print(f"  Bodies size: {BODIES_DB.stat().st_size / 1024 / 1024:.1f} MB")
