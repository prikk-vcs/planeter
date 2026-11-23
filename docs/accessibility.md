# Accessibility

**Status: a first draft.** This page says what planeter's web pages aim for, what has been checked and how,
what has not been checked, and what is known to be missing. It is **not a claim of conformance**: nobody has
audited planeter against the standard, and no person who uses assistive technology has tried it yet.

## The aim

The web UI aims for [WCAG 2.2](https://www.w3.org/TR/WCAG22/) level AA (RFC 012 D-1). Where a rule could be
checked by a program, the program is part of the test suite, so a change that breaks it fails the build.

## What has been checked, and how

Everything below was checked by tools, not by people.

- **Page structure, on every page and state the UI can produce** — a test (`crates/planeter-web/tests/structure/`)
  renders each page through the real router, parses it the way a browser does, and checks: the page language
  and text direction; exactly one top-level heading and no skipped heading level; a skip link that is the first
  thing reached by keyboard and leads to the main content; a header, main and footer landmark and a unique name
  for every navigation block; a visible label for every form field; a caption and header cells on every table
  the page itself builds; no icon or image without a text alternative; no hidden-but-focusable element; link
  text that is not a bare identifier or a word like "here" or "raw"; one title per kind of page, in one pattern,
  with long identifiers shortened; no `<form>` where HTML does not allow one; no inline script or style. Each
  rule has a deliberately broken page in the test, to prove the rule can fail.
- **Headings inside a README or other Markdown file** are moved one level down when the page is built, so the
  page keeps one top-level heading and the file's own headings nest below it. The sanitizer that makes
  repository content safe runs first; a test shows that hostile heading markup stays inert.
- **Text, words and translation** — every word on a page comes from one message catalog (RFC 012 D-3) and is
  checked by tests, which is what lets a screen reader and a translator find them.
- **The stylesheet** — a test (`crates/planeter-web/tests/stylesheet.rs`) reads `static/app.css` and fails on:
  a text colour pair below 4.5:1, or a control's border or the focus ring below 3:1, in either the light or the
  dark theme (every pair the rules use is computed, with thresholds written in the test, not read from the
  stylesheet); any physical-direction declaration (`margin-left`, `padding-right`, `left`, `text-align: left`,
  `float: right` …: logical properties only); an outline that is removed, a focus style that is thinner than 2 px,
  has no offset or does not cover links, buttons, inputs and the skip link; an animation or transition without a
  `prefers-reduced-motion` guard; a standalone link, button or input without a 24 × 24 CSS px minimum; a missing
  `color-scheme: light dark` or forced-colours block; `!important`; `overflow: hidden` used to hide overflow. Each
  check has a deliberately broken stylesheet in the test.
- **Reflow, text spacing and target size in a browser** (2026-10-23, headless Chromium 154 driven by
  puppeteer-core 25.14.0, over all 61 rendered states): no state scrolls sideways at 320 CSS px (the width a
  1280 px window has at 400 % zoom) or at 1280 px, with or without the WCAG 1.4.12 text-spacing override
  (line height 1.5, paragraph spacing 2×, letter spacing 0.12 em, word spacing 0.16 em); no standalone target is
  smaller than 24 × 24 CSS px; the focus ring was measured by pixel in both themes (light `#2a5db0` on white,
  6.38:1; dark `#7aa7ff` on `#111`, 7.91:1). This was measured once, by a script that is not part of the build.
- **Colour scheme and forced colours** were looked at in Chromium with `prefers-color-scheme` light and dark and
  `forced-colors: active` emulated, and with the test-only right-to-left language: the pages mirror and nothing
  overlaps. Screenshots are kept with the review.
- **The file-by-file view of a revision** (2026-11-17, handoff `review-view-pages-handoff-v1`): its pages are in the structural tests above
  (the changes page, one file's page and a page for each reason the view can be unavailable, in English and Japanese, over a real client, real
  prikk and the real router), the stylesheet tests (the diff rows' text on their backgrounds in both themes, a border style — solid for added,
  dashed for removed — that survives forced colours, so colour is never the only difference; the diff text keeps its spaces and wraps anywhere), and
  a browser measurement made once and not part of the build (Chromium 157 headless, 25 pages, light and dark, 320 and 1280 CSS px, with and without the
  WCAG 1.4.12 text-spacing override: no page scrolled sideways, no interactive target under 24 × 24 CSS px, every one of 822 focusable elements
  showed a focus ring). A name or a line of text from a proposal reaches a page only after the engine's display function, a second look at the page
  and the template's escaping; tests render a name with `<script>`, a line with `<img onerror>`, bidirectional overrides, zero-width characters, a
  2,500-character line and a symbolic-link target `javascript:…`, and none of them reaches a page unescaped or becomes a link.
- **The list of keys a repository trusts** (RFC 013, `docs/keys.md`): in the saved HTML it is a native `<details>` whose summary reads "Keys this repository trusts" and the number, and inside it a real `<ul>`; each item reads, in this order, the key's id, its fingerprint in eight groups and one sentence of where it came from, so a reader that goes item by item hears the same three things for every key. The structural rules pass over it in both languages for a repository with one key, one with every kind of source, one with the list unreadable, and one with 64 keys, and over the delivery line above it; the ids and fingerprints use the wrapping class the creation page uses (a stylesheet test holds it), so a long list is not wider than the screen. **What was not tried:** no assistive technology was run on it, and no browser measurement at 320 px was made for this section.
- **A one-off baseline** (2026-10-21) measured the pages in Chromium with axe-core 4.14.0 and html-validate
  11.17.0, scripted keyboard walks with puppeteer-core 25.14.0, colour contrast computed from the stylesheet
  (lowest ratio 4.87:1, passes), and the browser's accessibility tree. That run is where the list of gaps
  below comes from. It is not repeated on every change.

## The border rule

WCAG 1.4.11 asks for 3:1 on the parts of a control that are needed to see it. In planeter that means the
**border of a text field and of a button** (and of the skip link) — `--control` in the stylesheet, 4.54:1 on white
and 5.62:1 on the dark background — and the **focus ring** (`--accent`). Those are tested. The other lines are
**decorative** and are left as they are: the rules between table rows and under the header (`--line`, 1.36:1 and
1.49:1), the frame of a README and of code blocks, and the border of a plain badge; each is next to text that
carries the same information (a badge has its words; a row has its content), so nothing depends on seeing the line.
Status badges that are coloured (`ok`, `warn`, `bad`) draw their border in the same colour as their text, which is
checked as text at 4.5:1. The header's navigation links are told apart from text by **weight and an underline on
hover and on keyboard focus**, not by colour alone.

## What has not been checked

- **No testing with people who use assistive technology** — screen readers, screen magnifiers, switch or voice
  control. Whether the pages are *usable* with them, as opposed to passing the checks above, is unknown.
  In particular: how a screen reader reads the status badges and the tables row by row; whether a failed
  sign-in is noticed (the failure is announced as an alert, but that has not been heard).
- **The diff table with a screen reader.** It is one table with a caption, header cells, a hidden word for each row's change ("Added line",
  "Removed line", "Unchanged line") and a row for each block of lines; how it is *heard* — row by row, with the line numbers, in a long file — has
  not been tried, and is the first thing to try with a person who uses one. The same holds for the table of changed files.
- **Other browsers and systems.** The tool measurements used Chromium on Linux only; Firefox and Safari were
  not measured, nor the focus ring in them.
- **Forced colours on a real system** (Windows high-contrast with a real screen; the block was checked only
  with Chromium's emulation), **text-only zoom to 200 %, screen magnifiers, print output.**
- **Touch-target comfort on real devices** (every target is at least 24 × 24 CSS px, which is the minimum, not
  a comfortable size).
- **A real right-to-left language.** Only the test language was rendered; numbers and units inside a
  right-to-left sentence (`223 B`) have not been looked at by a reader of such a language.
- **Whether the wording is understood** — plain words, the sentences' tone, and any translation need a
  fluent human reader.
- **Voice control** ("label in name", WCAG 2.5.3) beyond what a script can compute.
- **Session expiry.** There is no warning before a sign-in expires, so there was nothing to measure.
- **Help (WCAG 3.2.6).** The pages offer no help or contact route of their own.

## Known gaps

These are known and planned, not fixed yet:

- A README's own links that are relative resolve against the page address, not the file's folder.
- Wording: the interface now uses the plain words of `docs/glossary.md` (approved by the owner), and the badges,
  *patch* and *rollback* are explained in place by a native expand/collapse block. No reader has tested whether the
  words are understood. One message is held unchanged until prikk confirms its meaning (the "N of M sessions
  verified" count).
- A repository whose only content is queued (unsigned) work shows an **empty page under a green "Verified" badge**,
  and nothing says that queued work exists (the badge is true: an empty sealed history verifies; the history page
  says "no signed changes"). prikk's `branch --format json` has a data field for it (`unpublished_current_branch`);
  the display waits for the queued-state design (IS-45, noted by the architect, not decided).
- **Languages.** English and **Japanese** are shipped. Japanese was reviewed by the owner (a fluent reader of Japanese; the colleague named in the
  plan did not take part) and signed off on 2026-10-31; the owner read the later messages and the Japanese version of this statement again and signed them off on 2026-11-14 (the language's record in `crates/planeter-i18n/src/languages/ja.rs` says who, when, the commit and the
  sheet); it is offered without any operator setting, chosen by `Accept-Language`, by the `planeter_lang` cookie, or by the switcher.
  The language switcher (a button per language in the footer of every page, no script) shows the languages (a menu from four on). A message written or changed **after** a sign-off ships and is flagged until a reviewer has read it: the number of such messages is printed by the server at start, shown on the review sheet and listed in each release's notes (`planeter-i18n-sheet --status`); this statement does not repeat a number that changes. What the checks cover for
  Japanese: every message exists with the same placeholders as English, no retired or forbidden term, the glossary's Japanese column
  equals the catalog, every page state passes the structural checker with `lang="ja"` (the stand-in and real-prikk states alike), and,
  measured in Chromium (2026-10-23, before the sign-off), none of the 61 states scrolls sideways at 320 px, with or without the WCAG
  text-spacing override, and no target is below 24 px. **No person who uses assistive technology has tried either language**, and a real
  right-to-left language is still not rendered. This statement has a Japanese version below, with the same facts.
- The focus ring is the same colour as links (it stands off the control by a 2 px gap); a designed ring for the
  few places where that is not enough has not been needed so far.

## How to report a problem

If a page does not work for you, or you find something here that is wrong, open an issue on the project's
repository (<https://github.com/prikk-vcs/planeter/issues>) and say which page you were on, what you were
using (browser, assistive technology, settings) and what happened. You do not need to know which rule it
breaks. A vulnerability is different: see [`SECURITY.md`](../SECURITY.md) and do not open a public issue.

## アクセシビリティ（日本語）

この節は、上の英語の内容と同じ事実を日本語で述べたものです。

**状況：初版の草案です。** このページは、planeter の Web ページが目指すこと、確認したこと、確認していないこと、すでに分かっている不足を述べます。**適合を主張するものではありません。** 標準に照らした監査は行われておらず、支援技術を使う人による試用もまだありません。

**目標。** Web UI は [WCAG 2.2](https://www.w3.org/TR/WCAG22/) のレベル AA を目指しています（RFC 012 D-1）。プログラムで確認できる規則は、テストスイートに組み込んであり、規則を破る変更はビルドで失敗します。

**確認したこと。** すべてツールによる確認で、人によるものではありません。
- ページの構造（UI が作れるすべてのページと状態）：言語と文字の向き、最上位の見出しがちょうど 1 つで見出しの段が飛ばないこと、キーボードで最初に届くスキップリンク、ヘッダー・メイン・フッターのランドマークと各ナビゲーションの固有の名前、すべての入力欄の見えるラベル、表の見出しと表題、代替テキストのない画像がないこと、「ここ」のような語だけのリンクがないこと、スクリプトやインラインスタイルがないこと。規則ごとに、わざと壊したページがテストにあり、規則が失敗しうることを示しています。
- 画面に出る言葉はすべて 1 つのメッセージカタログ（RFC 012 D-3）から来て、テストで確認されます。
- スタイルシート：文字色の組が 4.5:1 未満、操作部品の枠線またはフォーカスリングが 3:1 未満（明るいテーマと暗いテーマの両方）、物理方向の指定、消されたアウトライン、モーションを抑える設定のない動き、24 × 24 CSS px 未満の操作部品、などをテストが検出します。
- ブラウザーでの測定（2026-10-23、ヘッドレス Chromium、61 の状態）：320 CSS px でも 1280 px でも横にスクロールする状態はなく（WCAG 1.4.12 の文字間隔の上書きがあってもなくても）、24 × 24 CSS px 未満の操作部品もありません。これは 1 回だけ、ビルドに含まれないスクリプトで測定したものです。
- ファイルごとの変更点の表示（2026-11-17）：変更点のページ、1 つのファイルのページ、表示できない理由ごとのページを、日本語と英語で構造のテスト（実際のクライアントと prikk、実際のルーターを使用）とスタイルシートのテストに加えました（差分の行の文字と背景の組が明るいテーマでも暗いテーマでも基準を満たすこと、追加は実線・削除は破線の枠線で、強制カラーでも色だけに頼らずに区別できること、差分の文字が空白を保ったままどこでも折り返すこと）。ブラウザーでの測定は 1 回だけ、ビルドに含まれないスクリプトで行いました（ヘッドレス Chromium 157、25 ページ、明るいテーマと暗いテーマ、320 と 1280 CSS px、WCAG 1.4.12 の文字間隔の上書きがあってもなくても）。横にスクロールするページも、24 × 24 CSS px 未満の操作部品もなく、822 個のフォーカスできる要素のすべてでフォーカスの枠が見えました。提案に含まれる名前や本文は、サーバーの表示用の関数、ページ側の再確認、テンプレートのエスケープを通ってから表示されます。`<script>` を含む名前、`<img onerror>` を含む行、双方向制御文字、幅のない文字、2,500 文字の行、`javascript:` で始まるシンボリックリンクの先をテストで表示し、どれもそのまま表示されたり、リンクになったりしないことを確認しています。

**確認していないこと。**
- スクリーンリーダー、拡大鏡、スイッチ、音声操作など、支援技術を使う人による試用。バッジや表がどう読み上げられるか、ログインの失敗が気づかれるかは、分かっていません。
- スクリーンリーダーでの差分の表。キャプション、見出しセル、各行の「追加された行」「削除された行」「変わっていない行」の隠しテキスト、行のかたまりごとの見出し行を備えた 1 つの表ですが、実際にどう読み上げられるか（行ごと、行番号つき、長いファイルで）は試していません。支援技術を使う人と最初に試すべき点です。変わったファイルの一覧の表も同様です。
- Chromium（Linux）以外のブラウザーとシステム。実際のシステムでの強制カラー、文字だけの 200 % 拡大、印刷。
- 実機でのタッチ操作のしやすさ（すべての操作部品は 24 × 24 CSS px 以上ですが、これは最小値であり、快適な大きさではありません）。
- 実際の右から左へ書く言語（テスト用の言語だけを表示しました）。
- 言葉が理解されるかどうか。平易な言葉づかいと文の調子には、流暢な人間の読み手が必要です。日本語は、流暢に読めるオーナー 1 人が確認しましたが、利用者による確認ではありません。
- ログインの有効期限が切れる前の警告はありません（測るものがありませんでした）。ページ自身のヘルプや連絡先の案内もありません（WCAG 3.2.6）。

**問題の報告。** ページが使えない場合や、ここに誤りを見つけた場合は、プロジェクトのリポジトリ（<https://github.com/prikk-vcs/planeter/issues>）に Issue を開いてください。どのページで、何を使っていて（ブラウザー、支援技術、設定）、何が起きたかを書いてください。どの規則に反するかを知っている必要はありません。脆弱性は別です。[`SECURITY.md`](../SECURITY.md) を見て、公開の Issue は開かないでください。
