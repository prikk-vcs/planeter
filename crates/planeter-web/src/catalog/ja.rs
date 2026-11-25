//! The Japanese catalog (RFC 012 D-3, handoff `japanese-and-switcher-handoff-v1`): **signed off by its reviewers** (see
//! its record in the registry, `planeter_i18n::languages::ja`) and shipped like English. The sheet they reviewed is generated from this table
//! (`cargo run -p planeter-i18n-sheet -- --lang ja`, `docs/messages-and-languages.md`); a correction is an edit here and in `docs/glossary/ja.md`.
//!
//! Style: polite form (です・ます); headings, labels and badges are noun phrases; Japanese has no grammatical
//! plural, so a message with a count says the same in both forms; the order follows [`super::Key`] and the
//! English catalog, key for key, with the same placeholders. The terms are the Japanese column of
//! `docs/glossary/ja.md`; the **hard terms** are flagged there for the reviewers' decision.

use super::{Entry, msg};

/// The server's messages for this language.
pub static DEF: super::LangDef = super::LangDef::new(&planeter_i18n::languages::ja::JA, CATALOG);

pub const CATALOG: &[Entry] = &[
    // layout and navigation
    msg("layout.sign_in", "ログイン"),
    msg("layout.sign_out", "ログアウト"),
    msg("layout.skip", "本文へ移動"),
    msg("layout.nav_repository", "リポジトリ"),
    msg("layout.footer", "{product} {version}"),
    msg("layout.language", "言語"),
    msg("nav.code", "ファイル"),
    msg("nav.history", "履歴"),
    msg("nav.refs", "ブランチとタグ"),
    msg("nav.verify", "検証"),
    // page titles (one pattern: page · context · product)
    msg("title.home", "リポジトリを探す · {product}"),
    msg(
        "title.home_error",
        "リポジトリを探す · アドレスを認識できません · {product}",
    ),
    msg("title.busy", "混み合っています · {path} · {product}"),
    msg("title.login", "ログイン · {product}"),
    msg(
        "title.login_failed",
        "ログイン · 受け付けられませんでした · {product}",
    ),
    msg(
        "title.login_locked",
        "ログイン · 試行回数が多すぎます · {product}",
    ),
    msg("title.not_found", "見つかりません · {path} · {product}"),
    msg(
        "title.unavailable",
        "一時的に利用できません · {path} · {product}",
    ),
    msg(
        "title.too_large",
        "大きすぎて表示できません · {path} · {product}",
    ),
    msg("title.repo", "{repo} · {product}"),
    msg("title.tree", "{prefix} · {repo} · {product}"),
    msg("title.file", "{path} · {repo} · {product}"),
    msg("title.history", "履歴 · {repo} · {product}"),
    msg("title.change", "変更 {target} · {repo} · {product}"),
    msg("title.refs", "ブランチとタグ · {repo} · {product}"),
    msg("title.verify", "検証 · {repo} · {product}"),
    // home
    msg(
        "home.intro",
        "planeter は prikk のリポジトリをホストしています。リポジトリを開くには、{address} の形式でアドレスを入力してください。",
    ),
    msg("home.address_example", "owner/name"),
    msg("home.address_label", "リポジトリのアドレス"),
    msg(
        "home.address_error",
        "リポジトリのアドレスとして認識できません。{address} の形式で入力してください。",
    ),
    msg("home.open", "リポジトリを開く"),
    // sign-in
    msg("login.heading", "ログイン"),
    msg(
        "login.failed",
        "ユーザー名またはパスワードが正しくありません。両方を確認して、もう一度お試しください。",
    ),
    msg("login.sso", "シングルサインオンでログイン"),
    msg("login.user", "ユーザー名"),
    msg("login.password", "パスワード"),
    msg("login.submit", "ログイン"),
    msg(
        "login.locked",
        "ログインの試行回数が多すぎます。数分待ってから、もう一度お試しください。",
    ),
    // the plain pages
    msg("not_found.heading", "見つかりません"),
    msg(
        "not_found.body",
        "このアドレスにはリポジトリがないか、アクセスする権限がありません。アドレスを確認してください。非公開のリポジトリの場合は、ログインしてください。",
    ),
    msg("unavailable.heading", "一時的に利用できません"),
    msg(
        "unavailable.body",
        "今はこのリポジトリを読み込めません。しばらくしてから、もう一度お試しください。それでも解決しない場合は、このサーバーを運用している人にお知らせください。",
    ),
    msg("busy.heading", "サーバーが混み合っています"),
    msg(
        "busy.body",
        "現在、多くのリクエストを処理しています。しばらくしてから、もう一度お試しください。",
    ),
    msg("too_large.heading", "大きすぎて表示できません"),
    msg(
        "too_large.body",
        "これは大きすぎて、このサーバーでは表示できません。ダウンロードするか、このサーバーを運用している人に上限の引き上げを依頼してください。",
    ),
    // a missing path in a repository the reader may read (IS-45); the heading and title are `not_found.*`
    msg(
        "no_such_path.body",
        "このリポジトリには、このファイルまたはフォルダーが見つかりませんでした。パスを確認するか、ファイルの一覧に戻ってください。",
    ),
    msg("no_such_path.link", "ファイルの一覧へ"),
    // repository, tree, file
    msg("repo.location", "{point} を表示中"),
    msg(
        "repo.location_branch",
        "{point} を表示中 · ブランチ {branch}",
    ),
    msg("tree.at", "{point} を表示中"),
    msg("listing.size_and_encoding", "{size} · {encoding}"),
    msg("listing.caption", "ファイルとフォルダー"),
    msg("listing.name", "名前"),
    msg("listing.details", "サイズと種類"),
    msg("listing.folder", "フォルダー"),
    msg(
        "file.binary",
        "このファイルはテキストではないため、ここでは表示できません。{download}",
    ),
    msg("file.download_named", "{name} をダウンロード"),
    // sizes
    msg("size.bytes", "{value} B"),
    msg("size.kib", "{value} KiB"),
    msg("size.mib", "{value} MiB"),
    msg("size.gib", "{value} GiB"),
    // assurance labels (trust-bearing)
    msg("assurance.verified", "検証済み"),
    msg("assurance.sealed", "署名して履歴に記録済み"),
    msg("assurance.queued", "ここで保持中・未署名"),
    msg("assurance.unverified", "検証に失敗"),
    msg("assurance.verify_unavailable", "現在は確認できません"),
    // in-place explanations (a native <details>, no script)
    msg("explain.summary", "これはどういう意味ですか？"),
    msg(
        "explain.verified",
        "prikk がこのリポジトリに対してすべてのチェックをたった今実行し、問題は見つかりませんでした。",
    ),
    msg(
        "explain.signed",
        "メンテナーがこのパッチをプロジェクトの履歴に署名して記録しました。リポジトリ全体に問題がないかどうかは、「検証」ページで確認できます。",
    ),
    msg(
        "explain.queued",
        "このパッチは、このサーバーで保持されています。メンテナーがプロジェクトの履歴に署名していないため、検証済みの記録には含まれていません。",
    ),
    msg(
        "explain.failed",
        "prikk がこのリポジトリをチェックしたところ、問題が見つかりました。見つかった内容は「検証」ページに一覧があります。",
    ),
    msg(
        "explain.failed_here",
        "prikk がこのリポジトリをチェックしたところ、問題が見つかりました。内容は下に一覧で示します。",
    ),
    msg(
        "explain.unavailable",
        "しばらくしてから、もう一度お試しください。",
    ),
    msg(
        "explain.patch",
        "パッチとは、記録された編集 1 件のことです。",
    ),
    msg(
        "explain.rollback",
        "ロールバックとは、以前の変更を取り消す変更のことです。",
    ),
    // history, change, refs, verify
    msg("history.heading", "履歴 {ref}"),
    msg("history.rollback", "ロールバック"),
    msg("history.patch_count", "パッチ {count} 件"),
    msg(
        "history.empty",
        "このブランチには、署名済みの変更がまだありません。",
    ),
    msg("history.col.change", "変更"),
    msg("history.col.kind", "種類"),
    msg("history.col.patches", "パッチ"),
    msg("history.change_fallback", "変更 {id}"),
    msg("change.heading", "変更 {target}"),
    msg("change.patch", "パッチ {id}"),
    msg("change.col.operation", "操作"),
    msg("change.col.paths", "パス"),
    msg("refs.branches", "ブランチ"),
    msg("refs.tags", "タグ"),
    msg("refs.current", "現在"),
    msg("refs.closed", "閉じている"),
    msg("refs.no_tags", "タグはありません。"),
    msg("refs.heading", "ブランチとタグ"),
    msg("refs.col.branch", "ブランチ"),
    msg("refs.col.tag", "タグ"),
    msg("refs.col.state", "状態"),
    msg("refs.col.target", "対象"),
    msg("verify.heading", "検証"),
    msg(
        "verify.intro",
        "これらのチェックは、このページを開いたときにたった今実行されたものです。以前に開いたときの結果が再利用されることはありません。",
    ),
    msg("verify.findings", "見つかった内容"),
    msg("verify.status.evaluated", "完了"),
    msg("verify.status.failed", "問題あり"),
    msg("verify.status.not_evaluated", "未実行（{check} が原因）"),
    msg("verify.status.not_run", "未実行"),
    msg("verify.status.halted", "途中で停止"),
    msg("verify.status.halted_after", "途中で停止（{check} の後）"),
    msg("verify.col.stage", "チェック"),
    msg("verify.col.status", "状態"),
    msg("verify.col.detail", "詳細"),
    // the API's error messages and hints
    msg("error.not_found.message", "該当するリソースがありません。"),
    msg(
        "error.not_found.hint",
        "所有者、リポジトリ名、パスを確認してください。また、お使いのトークンにこのリポジトリを読む権限があることも確認してください。",
    ),
    msg(
        "error.unavailable.message",
        "今はこのリポジトリを読み込めません。",
    ),
    msg(
        "error.unavailable.hint",
        "しばらくしてから、もう一度お試しください。",
    ),
    msg("error.busy.message", "サーバーが混み合っています。"),
    msg(
        "error.busy.hint",
        "Retry-After ヘッダーに示された秒数が過ぎてから、再試行してください。",
    ),
    msg(
        "error.invalid_request.message",
        "リクエストの形式が正しくありません。",
    ),
    msg(
        "error.invalid_request.hint",
        "リクエストを、{document} にある OpenAPI ドキュメントと照らし合わせて確認してください。",
    ),
    msg(
        "error.method_not_allowed.message",
        "このメソッドは、ここではサポートされていません。",
    ),
    msg("error.method_not_allowed.hint", "GET を使用してください。"),
    msg(
        "error.raw_elsewhere.message",
        "ファイルのバイト列は、このアドレスからは提供されません。",
    ),
    msg(
        "error.raw_elsewhere.hint",
        "ファイルのページにあるリンク、または OpenAPI ドキュメントに記載されたコンテンツ配信元を使用してください。",
    ),
    msg(
        "error.too_large.message",
        "これは大きすぎて、ここでは提供できません。",
    ),
    msg(
        "error.too_large.hint",
        "コンテンツ配信元からダウンロードするか、運用者に読み取り用メモリの予算の引き上げを依頼してください。",
    ),
    msg(
        "error.too_large.raw_hint",
        "このサーバーは、そこまで大きなファイルを提供しません。このサーバーを運用している人に問い合わせてください。",
    ),
    msg(
        "error.internal.message",
        "サーバーがリクエストを完了できませんでした。",
    ),
    msg(
        "error.internal.hint",
        "後でもう一度お試しください。解決しない場合は、運用者にお知らせください。",
    ),
    msg(
        "error.transport_off.message",
        "このサーバーでは、アーティファクトの取得は設定されていません。",
    ),
    msg(
        "error.transport_off.hint",
        "このサーバーを運用している人に、エクスポート用スプールの設定を依頼してください。",
    ),
    msg(
        "error.unknown_ref.message",
        "その名前の ref は、このリポジトリにありません。",
    ),
    msg(
        "error.unknown_ref.hint",
        "ref の名前は、リポジトリのアドバタイズから確認してください。",
    ),
    msg(
        "error.unknown_kind.message",
        "その取得の種類はサポートされていません。",
    ),
    msg(
        "error.unknown_kind.hint",
        "kind=archive または kind=bundle を指定してください。",
    ),
    msg(
        "error.bundle_of_received_ref.message",
        "受信した ref は、バンドルとしては取得できません。",
    ),
    msg(
        "error.bundle_of_received_ref.hint",
        "代わりに kind=archive でリポジトリ全体を取得してください。",
    ),
    msg(
        "error.no_have_list.message",
        "その ref には、have リストがありません。",
    ),
    msg(
        "error.no_have_list.hint",
        "公開されたブランチの have リストを取得してください。名前はアドバタイズから確認できます。",
    ),
    msg(
        "error.not_verified.message",
        "エクスポートしたアーティファクトが検証に通らなかったため、提供しませんでした。",
    ),
    msg(
        "error.not_verified.hint",
        "しばらくしてから、もう一度お試しください。公開された ref は kind=bundle で取得できる場合があります。",
    ),
    msg(
        "error.export_failed.message",
        "今はアーティファクトをエクスポートできません。",
    ),
    msg(
        "error.export_failed.hint",
        "しばらくしてから、もう一度お試しください。",
    ),
    msg(
        "error.export_failed.deadline_hint",
        "エクスポートがこのサーバーの許す時間を超えました。このサーバーを運用している人に、エクスポートの制限時間の引き上げを依頼してください。",
    ),
    msg(
        "error.export_failed.ceiling_hint",
        "エクスポートに必要なメモリが、このサーバーの許す量を超えています。このサーバーを運用している人に、エクスポート用メモリ上限の引き上げを依頼してください。",
    ),
    msg(
        "error.export_failed.spool_hint",
        "アーティファクトがエクスポート用スプール全体より大きくなっています。このサーバーを運用している人に、スプールの拡大を依頼してください。",
    ),
    msg("error.rate_limited.message", "リクエストが多すぎます。"),
    msg(
        "error.rate_limited.hint",
        "Retry-After ヘッダーに示された秒数が過ぎてから、再試行してください。",
    ),
    msg(
        "error.range_not_satisfiable.message",
        "指定されたバイト範囲は提供できません。",
    ),
    msg(
        "error.range_not_satisfiable.hint",
        "アーティファクトの内側の範囲を指定してください。サイズは Content-Range ヘッダーに示されます。",
    ),
    msg(
        "error.invalid_request.ref_hint",
        "バンドルには ref が必要です。アドバタイズから名前を確認して、ref=名前 を追加してください。",
    ),
    // quota refusals of a proposal (RFC 005 D-9)
    msg(
        "quota.open_proposals.message",
        "このリポジトリで開いている提案がすでに {used} 件あり、上限は {limit} 件です。",
    ),
    msg(
        "quota.open_proposals.hint",
        "開いている提案のどれかを閉じるか、メンテナーに上限の引き上げを依頼してください。",
    ),
    msg(
        "quota.accepted_bytes.message",
        "このリポジトリにはすでに{used} MiB を送信済みで、上限は{limit} MiB です。",
    ),
    msg(
        "quota.accepted_bytes.hint",
        "送信した内容は提案を閉じた後もサーバーに残るため、メンテナーに上限の引き上げを依頼してください。",
    ),
    msg(
        "quota.revisions.message",
        "この提案にはすでに {used} 件のリビジョンがあり、上限は {limit} 件です。",
    ),
    msg(
        "quota.revisions.hint",
        "続きの作業は新しい提案として開くか、メンテナーに上限の引き上げを依頼してください。",
    ),
    msg(
        "quota.capacity.message",
        "このリポジトリは、提案を受け入れる容量の上限に達しています。{limit} MiB のうち {used} MiB が使用中です。",
    ),
    msg(
        "quota.capacity.hint",
        "メンテナーに完了した提案を閉じるよう依頼するか、このサーバーを運用している人に容量の引き上げを依頼してください。",
    ),
    // making a proposal (RFC 004 part 3b)
    msg(
        "error.forbidden.message",
        "このリポジトリは読めますが、提案する権限はありません。",
    ),
    msg(
        "error.forbidden.hint",
        "メンテナーに書き込み権限を依頼するか、ログインしている全員に提案を開放するよう依頼してください。",
    ),
    msg(
        "error.length_required.message",
        "提案の長さが示されていません。",
    ),
    msg(
        "error.length_required.hint",
        "Content-Length ヘッダーを付けて送信してください。",
    ),
    msg(
        "error.payload_too_large.message",
        "その提案は、このサーバーが受け付ける大きさを超えています。",
    ),
    msg(
        "error.payload_too_large.hint",
        "もっと小さいアーティファクトを送るか、このサーバーを運用している人に上限の引き上げを依頼してください。",
    ),
    msg(
        "error.needs_recovery.message",
        "このリポジトリは、完了しなかった書き込みの復旧を待っているため、今は新しい提案を受け付けません。",
    ),
    msg(
        "error.needs_recovery.hint",
        "しばらくしてからもう一度お試しください。読み取りには影響がなく、理由はこのサーバーを運用している人がサーバーのログで確認できます。",
    ),
    msg(
        "error.quota_exceeded.message",
        "このリポジトリへの提案の上限に達しました。",
    ),
    msg(
        "error.quota_exceeded.hint",
        "詳細はこのレスポンスに含まれているので、提案を閉じるか、メンテナーに上限の引き上げを依頼してください。",
    ),
    msg(
        "error.base_unknown.message",
        "指定されたベースは、このリポジトリの履歴にありません。",
    ),
    msg(
        "error.base_unknown.hint",
        "先にベースを送るか、ベースの指定を省いてください。",
    ),
    msg(
        "error.declaration_misses_added.message",
        "宣言されたパッチの一覧に、このアーティファクトが追加したパッチが含まれていません。",
    ),
    msg(
        "error.declaration_misses_added.hint",
        "アーティファクトはすでにこのサーバーにあるので、同じアーティファクトを、修正した一覧とともにもう一度送信してください。",
    ),
    msg(
        "error.declared_not_held.message",
        "宣言されたパッチの一覧に、このリポジトリが持っていないパッチが含まれています。",
    ),
    msg(
        "error.declared_not_held.hint",
        "アーティファクトはすでにこのサーバーにあるので、同じアーティファクトを、修正した一覧とともにもう一度送信してください。",
    ),
    msg(
        "error.no_new_content.message",
        "このアーティファクトには、リポジトリがまだ持っていないものが含まれていません。",
    ),
    msg(
        "error.no_new_content.hint",
        "提案するものがないので、何かを変更した場合はアーティファクトを作り直してください。",
    ),
    msg(
        "error.unknown_proposal.message",
        "このリポジトリに、その ID の提案はありません。",
    ),
    msg(
        "error.unknown_proposal.hint",
        "以前のレスポンスから ID を取るか、ID を省いて新しい提案を開いてください。",
    ),
    msg(
        "error.proposal_not_open.message",
        "その提案は反映済みか閉じられているため、これ以上リビジョンを受け付けません。",
    ),
    msg(
        "error.proposal_not_open.hint",
        "代わりに新しい提案を開いてください。",
    ),
    msg(
        "error.not_your_proposal.message",
        "その提案は別の人が開いたものです。",
    ),
    msg(
        "error.not_your_proposal.hint",
        "自分の提案を開くか、メンテナーに相談してください。",
    ),
    msg(
        "error.accept_refused.message",
        "prikk がこのアーティファクトを拒否したため、リポジトリには何も追加されていません。",
    ),
    msg(
        "error.accept_refused.hint",
        "detail フィールドの理由を読み、アーティファクトを直してからもう一度送信してください。",
    ),
    msg(
        "error.intent_pending.message",
        "別の人がこのアーティファクトの送信をすでに始めていて、まだ完了していません。",
    ),
    msg(
        "error.intent_pending.hint",
        "リポジトリが復旧してから、もう一度お試しください。",
    ),
    msg(
        "error.incomplete.message",
        "リポジトリはアーティファクトの取り込みを完了できませんでした。",
    ),
    msg(
        "error.incomplete.hint",
        "リポジトリが復旧してから、同じアーティファクトをもう一度送信してください。",
    ),
    msg(
        "error.invalid_request.framing_hint",
        "本文は PLANETER-PROPOSE-1 で始め、続けてメタデータの長さ、メタデータ、アーティファクトの順に並べてください。",
    ),
    msg(
        "error.invalid_request.metadata_hint",
        "フィールドの一覧は API ドキュメントにあるので、reason に示されたフィールドを直してから、提案をもう一度送信してください。",
    ),
    msg(
        "error.forbidden_admin.message",
        "このリポジトリは読めますが、管理する権限はありません。",
    ),
    msg(
        "error.forbidden_admin.hint",
        "リポジトリの管理者に依頼してください。",
    ),
    msg(
        "error.propose_off.message",
        "このサーバーでは、提案の受け付けは設定されていません。",
    ),
    msg(
        "error.propose_off.hint",
        "このサーバーを運用している人に、アーティファクト保存先の設定を依頼してください。",
    ),
    msg(
        "propose.held",
        "あなたの作業はこのサーバーでレビュー待ちとして保持されており、どのブランチも変わっていません。",
    ),
    msg(
        "propose.already_recorded",
        "このアーティファクトはすでに記録済みのため何も追加されておらず、どのブランチも変わっていません。",
    ),
    // recovery (RFC 004 part 4)
    msg(
        "recovery.cause.accept_deadline",
        "取り込みがこのサーバーの許す時間を超えたため、停止されました。",
    ),
    msg(
        "recovery.cause.accept_ceiling",
        "取り込みがこのサーバーの許すメモリ量を超えたため、停止されました。",
    ),
    msg(
        "recovery.cause.accept_failed",
        "取り込みが途中で失敗しました。",
    ),
    msg(
        "recovery.cause.accept_unreadable",
        "取り込みは終了しましたが、出力を読み取れませんでした。",
    ),
    msg(
        "recovery.cause.accept_not_recorded",
        "取り込みは終了しましたが、その記録を保存できませんでした。",
    ),
    msg(
        "recovery.cause.server_stopped",
        "書き込みの途中でサーバーが停止しました。",
    ),
    msg(
        "recovery.cause.delivery_unfinished",
        "引き渡しの実行中にサーバーが停止しました。",
    ),
    msg(
        "recovery.cause.other",
        "このサーバーが認識しない理由でフラグが立てられています。",
    ),
    msg(
        "recovery.blocker.cause_not_ours",
        "フラグは未完了の取り込みによるものではないため、自動では解除されません。",
    ),
    msg(
        "recovery.blocker.lease_held",
        "リポジトリのリースが現在使用中です。",
    ),
    msg(
        "recovery.blocker.foreign_process",
        "このサーバーのものではないプロセスが、リポジトリを使用しています。",
    ),
    msg(
        "recovery.blocker.doctor_problems",
        "リポジトリの点検で、人の対応が必要な問題が見つかりました。",
    ),
    msg(
        "recovery.blocker.output_unreadable",
        "リポジトリのツールの出力を読み取れなかったため、何も変えていません。",
    ),
    msg(
        "recovery.blocker.unexpected_lock",
        "想定外の種類のロック、またはリポジトリの外のロックが見つかりました。",
    ),
    msg(
        "recovery.blocker.too_many_open_intents",
        "リースの記録で説明できる数より多くの提案が待機しています。",
    ),
    msg(
        "recovery.blocker.processes_uninspected",
        "一部のプロセスを調べられなかったため、そのどれかがリポジトリを使用している可能性を除外できません。",
    ),
    msg(
        "recovery.note.unreached_objects",
        "リポジトリの点検で、どこからも使われていない保存データが {count} 件見つかりました。復旧は止まらず、対応は不要です。",
    ),
    msg(
        "recovery.outcome.recovered",
        "リポジトリは再び書き込めるようになりました。",
    ),
    msg(
        "recovery.outcome.stopped",
        "復旧は停止し、これ以上は何も変えていません。フラグはまだ立っています。",
    ),
    msg(
        "recovery.outcome.not_now",
        "リポジトリは現在使用中です。しばらくしてからもう一度お試しください。",
    ),
    msg(
        "recovery.outcome.nothing_to_do",
        "リポジトリは復旧待ちではありませんでした。",
    ),
    msg(
        "error.unknown_intent.message",
        "それはこのリポジトリの待機中の提案ではありません。",
    ),
    msg(
        "error.unknown_intent.hint",
        "復旧計画にある待機中の提案から ID を取ってください。",
    ),
    // pages that used to be bare strings (T7): a heading, one sentence, one link
    msg("page.link.sign_in", "ログインのページへ"),
    msg("page.link.home", "ホームページへ"),
    msg("page.template_error", "テンプレートのエラー"),
    msg(
        "page.csrf.title",
        "リクエストは受け付けられませんでした · {product}",
    ),
    msg("page.csrf.heading", "リクエストは受け付けられませんでした"),
    msg(
        "page.csrf.body",
        "このフォームがあなたのものであることを確認できなかったため、何も行われていません。前のページに戻り、ページを再読み込みして、もう一度お試しください。",
    ),
    msg(
        "page.form_incomplete.title",
        "フォームの入力が足りません · {product}",
    ),
    msg("page.form_incomplete.heading", "フォームの入力が足りません"),
    msg(
        "page.form_incomplete.body",
        "空の項目があったため、何も行われていません。前のページに戻って、入力してください。",
    ),
    msg(
        "page.sso_off.title",
        "シングルサインオンは設定されていません · {product}",
    ),
    msg(
        "page.sso_off.heading",
        "シングルサインオンは設定されていません",
    ),
    msg(
        "page.sso_off.body",
        "このサーバーには、シングルサインオンのプロバイダーが設定されていません。代わりに、ユーザー名でログインしてみてください。",
    ),
    msg(
        "page.sso_down.title",
        "シングルサインオンは現在利用できません · {product}",
    ),
    msg(
        "page.sso_down.heading",
        "シングルサインオンは現在利用できません",
    ),
    msg(
        "page.sso_down.body",
        "ログインのプロバイダーに接続できませんでした。代わりに、ユーザー名でログインしてみてください。",
    ),
    msg(
        "page.sso_incomplete.title",
        "ログインが完了しませんでした · {product}",
    ),
    msg(
        "page.sso_incomplete.heading",
        "ログインが完了しませんでした",
    ),
    msg(
        "page.sso_incomplete.body",
        "ログインのプロバイダーからの応答が不完全でした。代わりに、ユーザー名でログインしてみてください。",
    ),
    msg(
        "page.sso_mismatch.title",
        "ログインが一致しませんでした · {product}",
    ),
    msg("page.sso_mismatch.heading", "ログインが一致しませんでした"),
    msg(
        "page.sso_mismatch.body",
        "このログインを、開始したリクエストと結び付けることができませんでした。代わりに、ユーザー名でログインしてみてください。",
    ),
    msg(
        "page.sso_expired.title",
        "ログインの有効期限が切れました · {product}",
    ),
    msg("page.sso_expired.heading", "ログインの有効期限が切れました"),
    msg(
        "page.sso_expired.body",
        "ログインに時間がかかりすぎて、有効期限が切れました。代わりに、ユーザー名でログインしてみてください。",
    ),
    msg(
        "page.sso_refused.title",
        "ログインは受け付けられませんでした · {product}",
    ),
    msg(
        "page.sso_refused.heading",
        "ログインは受け付けられませんでした",
    ),
    msg(
        "page.sso_refused.body",
        "ログインのプロバイダーが、このログインを受け付けませんでした。代わりに、ユーザー名でログインしてみてください。",
    ),
    msg(
        "page.sso_unlinked.title",
        "このアカウントは連携されていません · {product}",
    ),
    msg(
        "page.sso_unlinked.heading",
        "このアカウントは連携されていません",
    ),
    msg(
        "page.sso_unlinked.body",
        "このシングルサインオンの ID は、planeter のアカウントに連携されていません。このサーバーを運用している人に問い合わせてください。",
    ),
    msg(
        "page.sso_disabled.title",
        "このアカウントは無効になっています · {product}",
    ),
    msg(
        "page.sso_disabled.heading",
        "このアカウントは無効になっています",
    ),
    msg(
        "page.sso_disabled.body",
        "このアカウントは無効になっています。このサーバーを運用している人に問い合わせてください。",
    ),
    msg("page.server_error.title", "問題が発生しました · {product}"),
    msg("page.server_error.heading", "問題が発生しました"),
    msg(
        "page.server_error.body",
        "サーバーで問題が発生し、ログインできませんでした。しばらくしてから、もう一度お試しください。",
    ),
    // proposals you can see (RFC 005 part 5)
    msg(
        "error.reason_required.message",
        "提案を閉じるには理由が必要ですが、理由が指定されていません。",
    ),
    msg(
        "error.reason_required.hint",
        "1〜2文の理由を添えて、もう一度送信してください。",
    ),
    msg(
        "error.proposal_final.message",
        "その提案はすでに反映済みか閉じられているため、もう一度閉じることはできません。",
    ),
    msg(
        "error.proposal_final.hint",
        "これ以上は何もできません。続きの作業は新しい提案として開いてください。",
    ),
    msg(
        "error.artifact_removed.message",
        "そのリビジョンのファイルは{date}まで保管され、その後に削除されました。",
    ),
    msg(
        "error.artifact_removed.hint",
        "まだ必要な場合は、提案した人にもう一度送ってもらうよう依頼してください。",
    ),
    msg(
        "error.changes_unavailable.partly_landed.message",
        "このリビジョンの作業は、一部がプロジェクトのブランチに反映済みで、一部はまだここでレビューを待っているため、1 つの変更としては表示できません。",
    ),
    msg(
        "error.changes_unavailable.partly_landed.hint",
        "プロジェクトの履歴で、反映された内容をご覧ください。残りは、まだレビュー待ちです。",
    ),
    msg(
        "error.changes_unavailable.claim_not_held.message",
        "このリビジョンの変更点は、もうここでは表示できません。",
    ),
    msg(
        "error.changes_unavailable.claim_not_held.hint",
        "このリビジョンの作業はもうレビュー待ちとしてここに残っていないため、プロジェクトの履歴を見るか、リビジョンのファイルをダウンロードして、自分のコンピューターで読んでください。",
    ),
    msg(
        "error.changes_unavailable.not_one_change.message",
        "このリビジョンは一直線の作業ではないため、ファイルごとに表示できません。",
    ),
    msg(
        "error.changes_unavailable.not_one_change.hint",
        "表示できるのは、枝分かれのない一直線の作業だけです。リビジョンのファイルをダウンロードして、自分のコンピューターで読んでください。",
    ),
    msg(
        "error.changes_unavailable.tree_too_large.message",
        "このリビジョンはファイルが多すぎるか大きすぎるため、ファイルごとに表示できません。",
    ),
    msg(
        "error.changes_unavailable.tree_too_large.hint",
        "リビジョンのファイルをダウンロードして、自分のコンピューターで読んでください。",
    ),
    msg(
        "error.changes_unavailable.busy.message",
        "現在、サーバーはファイルごとの表示で混み合っています。",
    ),
    msg(
        "error.changes_unavailable.busy.hint",
        "しばらくしてからもう一度お試しください。",
    ),
    msg(
        "error.changes_unavailable.timed_out.message",
        "ファイルごとの表示に時間がかかりすぎたため、中止しました。",
    ),
    msg(
        "error.changes_unavailable.timed_out.hint",
        "あとでもう一度お試しいただき、何度も起きる場合は、リビジョンのファイルをダウンロードして、自分のコンピューターで読んでください。",
    ),
    msg(
        "error.changes_unavailable.failed.message",
        "ファイルごとに表示できませんでした。",
    ),
    msg(
        "error.changes_unavailable.failed.hint",
        "何も変わっていないので、あとでもう一度お試しください（原因は、サーバーを運用している人がサーバーのログで確認できます）。",
    ),
    msg(
        "error.forbidden_review.message",
        "このリポジトリは読めますが、提案された内容をダウンロードできるのは、ここでレビューまたはマージを行う人だけです。",
    ),
    msg(
        "error.forbidden_review.hint",
        "メンテナーにそのアクセス権を依頼してください。",
    ),
    msg(
        "error.forbidden_close.message",
        "このリポジトリは読めますが、この提案を閉じられるのは、提案した本人かメンテナーだけです。",
    ),
    msg(
        "error.forbidden_close.hint",
        "その人に閉じてもらうか、メンテナーに書き込み権限を依頼してください。",
    ),
    msg("nav.proposals", "提案"),
    msg("title.proposals", "提案 · {repo} · {product}"),
    msg("title.proposal", "{title} · 提案 · {repo} · {product}"),
    msg("proposals.heading", "提案"),
    msg(
        "proposals.intro",
        "このリポジトリでレビューのために提出された作業です。",
    ),
    msg(
        "proposals.no_merge",
        "このサーバーは何もマージしません。メンテナーが自分のコンピューターで作業に署名します。",
    ),
    msg("proposals.filter.label", "表示する提案"),
    msg("proposals.filter.all", "すべて"),
    msg("proposals.empty", "表示できる提案はありません。"),
    msg("proposals.col.proposal", "提案"),
    msg("proposals.col.by", "提案者"),
    msg("proposals.col.state", "状態"),
    msg("proposals.col.updated", "最終更新"),
    msg("proposals.col.summary", "概要"),
    msg("proposals.more", "さらに提案を表示"),
    msg("proposal.state.open", "開いている"),
    msg("proposal.state.landed", "反映済み"),
    msg("proposal.state.closed", "閉じている"),
    msg(
        "proposal.what.open",
        "レビュー待ちで保持中：まだプロジェクトの履歴には含まれていません。",
    ),
    msg(
        "proposal.what.landed",
        "反映済み：メンテナーがこの作業をプロジェクトの履歴に署名して記録しました。",
    ),
    msg(
        "proposal.what.closed",
        "閉じています：この提案がプロジェクトの履歴に含まれることはありません。",
    ),
    msg("proposal.summary", "{revisions}にパッチ {count} 件"),
    msg("proposal.revisions_count", "リビジョン {count} 件"),
    msg("proposal.by", "提案者：{who}"),
    msg("proposal.when", "最終更新：{when}"),
    msg("proposal.closed_reason", "理由：{reason}"),
    msg("proposal.details", "詳細"),
    msg("proposal.description", "説明"),
    msg("proposal.no_description", "説明は書かれていません。"),
    msg("changes.link", "リビジョン {seq} の変更点を見る"),
    msg(
        "title.changes",
        "提案 #{id} のリビジョン {seq} の変更点 · {repo} · {product}",
    ),
    msg("changes.heading", "提案 #{id} のリビジョン {seq} の変更点"),
    msg(
        "changes.statement.landed",
        "この変更は反映済みです。プロジェクトのブランチの一部になっています。",
    ),
    msg(
        "changes.statement.rebuilt",
        "ここに表示するファイルは、このリビジョンのパッチから再構築したものです。ここに表示した内容は、署名されたものではありません。",
    ),
    msg(
        "changes.statement.provisional",
        "このリポジトリの履歴を以前に確認した記録がなかったため、履歴を最初から再現してファイルを再構築しました。ファイルはパッチの内容と一致しますが、履歴そのものは確認されていません。",
    ),
    msg(
        "changes.statement.base_differs",
        "このリビジョンは {declared} を土台にしたと述べていますが、最初のパッチは {real} から続いています。以下の変更点は {real} と比べたものです。",
    ),
    msg(
        "changes.statement.base_differs_none",
        "このリビジョンは {declared} を土台にしたと述べていますが、最初のパッチは履歴の最初にあたるため、比べる以前のファイルはありません。",
    ),
    msg(
        "changes.statement.starts",
        "このリビジョンは履歴の最初にあたるため、比べる以前のファイルはありません。",
    ),
    msg(
        "changes.statement.compared",
        "{before} と比べています（最初のパッチが続いている、履歴上の地点です）。",
    ),
    msg("changes.ids.summary", "完全な ID"),
    msg("changes.ids.before", "比べた相手：{id}"),
    msg("changes.ids.after", "このリビジョンの終点：{id}"),
    msg("changes.summary.files", "変わったファイル {count} 件"),
    msg("changes.summary.added", "追加 {count} 行"),
    msg("changes.summary.removed", "削除 {count} 行"),
    msg("changes.summary", "{files}、{added}、{removed}"),
    msg(
        "changes.summary.unshown",
        "そのうち {number} 件は、テキストとして表示できません（「補足」列を参照）。",
    ),
    msg(
        "changes.none",
        "このリビジョンで変わるファイルはありません。",
    ),
    msg(
        "changes.table.caption",
        "リビジョン {seq} で変わったファイル",
    ),
    msg("changes.col.file", "ファイル"),
    msg("changes.col.change", "変更の種別"),
    msg("changes.col.lines", "行数"),
    msg("changes.col.notes", "補足"),
    msg("changes.status.added", "追加"),
    msg("changes.status.modified", "変更"),
    msg("changes.status.deleted", "削除"),
    msg("changes.status.type_changed", "種類の変更"),
    msg("changes.row.lines", "追加 {added} 行、削除 {removed} 行"),
    msg("changes.row.lines_none", "行数なし"),
    msg(
        "changes.note.too_large",
        "ファイルが大きすぎるため、ここには表示しません。",
    ),
    msg(
        "changes.note.not_utf8",
        "UTF-8 のテキストではないため、テキストとして表示できません。",
    ),
    msg(
        "changes.note.binary",
        "バイナリファイルのため、テキストとして表示できません。",
    ),
    msg(
        "changes.note.symlink",
        "ファイルとシンボリックリンクの間で種類が変わったため、テキストとして表示できません。",
    ),
    msg(
        "changes.note.directory",
        "フォルダーであるか、フォルダーに変わったため、表示するテキストがありません。",
    ),
    msg(
        "changes.note.unreadable",
        "通常のファイルではないか、階層が深すぎるため、何も表示しません。",
    ),
    msg(
        "changes.note.link",
        "シンボリックリンクです。リンク先を表示しています。",
    ),
    msg("changes.earlier", "ここより前のファイル"),
    msg("changes.more", "ここより後のファイル"),
    msg(
        "title.changes_file",
        "{path} の変更点（リビジョン {seq}） · {repo} · {product}",
    ),
    msg(
        "changes.file.escaped",
        "このファイルの名前には特殊な文字が含まれており、バックスラッシュで始まるエスケープ表記で示しています。",
    ),
    msg("changes.file.status", "変更の種別：{status}"),
    msg("changes.file.caption", "{path} の変更点"),
    msg("changes.diff.col.old", "変更前の行"),
    msg("changes.diff.col.new", "変更後の行"),
    msg("changes.diff.col.text", "テキスト"),
    msg("changes.hunk", "{before}、{after}"),
    msg("changes.hunk.before_one", "変更前：{from} 行目"),
    msg("changes.hunk.before_many", "変更前：{from}～{to} 行目"),
    msg("changes.hunk.before_none", "変更前：なし"),
    msg("changes.hunk.after_one", "変更後：{from} 行目"),
    msg("changes.hunk.after_many", "変更後：{from}～{to} 行目"),
    msg("changes.hunk.after_none", "変更後：なし"),
    msg("changes.row.added", "追加された行"),
    msg("changes.row.removed", "削除された行"),
    msg("changes.row.same", "変わっていない行"),
    msg(
        "changes.file.truncated",
        "ほかに {count} 行は表示していません。",
    ),
    msg(
        "changes.file.approximate",
        "このファイルは変わった部分が多いため、古い行をすべて削除し、新しい行をすべて追加したものとして表示しています。",
    ),
    msg(
        "changes.file.cut_lines",
        "長すぎる行は途中で切ってあり、切った箇所に省いた量を示しています。",
    ),
    msg(
        "changes.cut.chars",
        "…（あと {count} 文字は表示されません）",
    ),
    msg(
        "changes.cut.bytes",
        "…（名前のあと {count} バイトは表示されません）",
    ),
    msg("changes.file.empty", "このファイルには行がありません。"),
    msg(
        "changes.link.added",
        "{target} へのシンボリックリンクです。",
    ),
    msg(
        "changes.link.deleted",
        "{target} へのシンボリックリンクでした。",
    ),
    msg(
        "changes.link.modified",
        "{target} へのシンボリックリンクです。以前は {old} を指していました。",
    ),
    msg(
        "changes.file.download",
        "読むにはリビジョン {seq} をダウンロード",
    ),
    msg("changes.file.back", "ファイルの一覧に戻る"),
    msg("changes.file.prev", "前の変更されたファイルへ"),
    msg("changes.file.next", "次の変更されたファイルへ"),
    msg(
        "title.changes_unavailable",
        "{heading} · 提案 #{id} のリビジョン {seq} · {repo} · {product}",
    ),
    msg(
        "changes.unavailable.partly_landed.heading",
        "この変更点は、1 つの変更としては表示できません",
    ),
    msg(
        "changes.unavailable.partly_landed.body",
        "このリビジョンの作業は、一部がプロジェクトのブランチに反映済みで、一部はまだここでレビューを待っています。反映された内容は、プロジェクトの履歴でご覧ください。残りは、まだレビュー待ちです。",
    ),
    msg(
        "changes.unavailable.claim_not_held.heading",
        "この変更点は、ここではもう表示できません",
    ),
    msg(
        "changes.unavailable.claim_not_held.body",
        "このリビジョンの作業は、もう提案としてここには残っていません。たとえば、署名された履歴として届いた場合です。代わりに、リポジトリの履歴をご覧ください。",
    ),
    msg(
        "changes.unavailable.not_one_change.heading",
        "このリビジョンはファイルごとに表示できません",
    ),
    msg(
        "changes.unavailable.not_one_change.body",
        "このリビジョンのパッチは一直線につながっていない（枝分かれや合流がある）ため、ファイルごとに表示できません。提案した人に、一直線につながる形で送り直してもらってください。",
    ),
    msg(
        "changes.unavailable.tree_too_large.heading",
        "このリビジョンは大きすぎて表示できません",
    ),
    msg(
        "changes.unavailable.tree_too_large.body",
        "このリビジョンはファイルが多すぎるか、大きすぎるため、ファイルごとに表示できません。代わりに、自分のコンピューターで読んでください。",
    ),
    msg(
        "changes.unavailable.busy.heading",
        "サーバーが混み合っています",
    ),
    msg(
        "changes.unavailable.busy.body",
        "サーバーは今、ほかの変更点を表示する処理で混み合っています。しばらくしてから、もう一度お試しください。",
    ),
    msg(
        "changes.unavailable.timed_out.heading",
        "表示に時間がかかりすぎました",
    ),
    msg(
        "changes.unavailable.timed_out.body",
        "変更点の表示に時間がかかりすぎたため、中止しました。しばらくしてからもう一度お試しいただき、何度も起きる場合は、自分のコンピューターで読んでください。",
    ),
    msg(
        "changes.unavailable.failed.heading",
        "変更点を表示できませんでした",
    ),
    msg(
        "changes.unavailable.failed.body",
        "このリビジョンの変更点を表示できませんでした。何も変わっていません。しばらくしてからもう一度お試しいただき、何度も起きる場合は、このサーバーを運用している人に伝えてください。",
    ),
    msg("proposal.revisions.heading", "リビジョン"),
    msg("proposal.revisions.col.revision", "リビジョン"),
    msg("proposal.revisions.col.by", "送信者"),
    msg("proposal.revisions.col.sent", "送信日時"),
    msg("proposal.revisions.col.added", "追加されたパッチ"),
    msg("proposal.revisions.col.built_on", "土台とした履歴"),
    msg("proposal.revisions.col.declared", "パッチの一覧"),
    msg("proposal.revisions.col.files", "ファイル"),
    msg("proposal.revision.declared", "送信者が一覧を示しました"),
    msg(
        "proposal.revision.undeclared",
        "送信者が一覧を示していないため、パッチが不完全な可能性があります",
    ),
    msg("proposal.revision.no_base", "土台は示されていません"),
    msg(
        "proposal.revision.claims",
        "このサーバーが確認した署名の主張",
    ),
    msg("proposal.revision.keys", "パッチに署名した鍵"),
    msg(
        "proposal.revision.download",
        "リビジョン {seq} をダウンロード",
    ),
    msg("proposal.revision.removed", "削除済み"),
    msg("proposal.patches.heading", "パッチ"),
    msg(
        "proposal.patches.pending",
        "{count} 件のパッチは、まだこのサーバーが保持しており、署名されていません。",
    ),
    msg(
        "proposal.patches.truncated",
        "最初の {limit} 件のパッチを表示しています。",
    ),
    msg("proposal.patches.col.patch", "パッチ"),
    msg("proposal.patches.col.status", "状態"),
    msg("proposal.patch.unknown", "判断できません"),
    msg(
        "proposal.patch.unknown_explain",
        "このサーバーは、このパッチが署名されて履歴に入ったかどうかを判断できません（履歴が、サーバーが読む範囲より長いためです）。",
    ),
    msg("proposal.patch.not_listed", "一覧にありません"),
    msg(
        "proposal.patch.not_listed_explain",
        "このパッチに該当するものは、このサーバーの、未署名のパッチにもプロジェクトの履歴にも見つかりません。そのためパッチの状態を表示できません。",
    ),
    msg(
        "proposal.retention.kept",
        "この提案のファイルは{date}まで保管されます。",
    ),
    msg(
        "proposal.retention.partly",
        "この提案のファイルの一部は{date}まで保管され、残りは削除されています。",
    ),
    msg(
        "proposal.retention.removed",
        "この提案のファイルは{date}まで保管され、その後に削除されました。",
    ),
    msg("proposal.close.heading", "この提案を閉じる"),
    msg("proposal.close.reason", "閉じる理由"),
    msg("proposal.close.button", "提案を閉じる"),
    msg(
        "proposal.close.consequence",
        "閉じても記録とファイルは残りますが、誰も再び開くことはできません。",
    ),
    msg(
        "proposal.download",
        "リビジョン {seq} のファイルをダウンロード",
    ),
    msg("repo.open_proposals", "開いている提案 {count} 件"),
    msg("proposal.back", "提案に戻る"),
    msg(
        "proposal.close.too_long",
        "理由が 1,000 文字を超えています。短くして、もう一度お試しください。",
    ),
    msg("proposal.closed_line", "理由：{reason}"),
    msg(
        "proposal.sighting",
        "ここで受信した履歴にも見つかりました（ここでは未検証）：{ref}",
    ),
    msg(
        "proposal.sighting_explain",
        "ここで受信した履歴は置き換えられることがあるため、この作業が反映済みであることを示すものではありません。",
    ),
    msg(
        "error.comments_closed.message",
        "この提案へのコメントは、完了後の議論の期間が終わった {date} に締め切られました。",
    ),
    msg(
        "error.comments_closed.hint",
        "続きの話は、新しい提案として始めてください。",
    ),
    msg(
        "error.decisions_closed.message",
        "この提案は反映済みか閉じられているため、これ以上レビューの判断を受け付けません。",
    ),
    msg(
        "error.decisions_closed.hint",
        "判断を記録できるのは提案が開いている間だけです。コメントはしばらくの間続けられます。",
    ),
    msg(
        "error.revision_not_current.message",
        "判断は最新のリビジョンに対して行います。最新はリビジョン {current} です。",
    ),
    msg(
        "error.revision_not_current.hint",
        "リビジョンを省くと最新のリビジョンへの判断になります。以前のリビジョンは履歴です。",
    ),
    msg(
        "error.self_approval.message",
        "自分が送った作業を自分で承認することはできません。",
    ),
    msg(
        "error.self_approval.hint",
        "ほかの人にレビューを依頼してください（変更の依頼やコメントはできます）。",
    ),
    msg(
        "error.comment_state.message",
        "そのコメントはすでに指定の状態になっています。",
    ),
    msg(
        "error.comment_state.hint",
        "提案を読み込み直して、コメントの現在の状態を確認してください。",
    ),
    msg(
        "error.forbidden_comment.message",
        "このリポジトリは読めますが、コメントするにはログインが必要です。",
    ),
    msg(
        "error.forbidden_comment.hint",
        "ログインするか、アクセストークンを付けてリクエストを送ってください。",
    ),
    msg(
        "error.forbidden_decision.message",
        "このリポジトリは読めますが、判断を記録できるのはここでレビューできる人だけです。",
    ),
    msg(
        "error.forbidden_decision.hint",
        "メンテナーにその権限を依頼してください。",
    ),
    msg(
        "error.forbidden_hide.message",
        "このリポジトリは読めますが、コメントを非表示にしたり再表示したりできるのはリポジトリの管理者だけです。",
    ),
    msg(
        "error.forbidden_hide.hint",
        "リポジトリの管理者に依頼してください。",
    ),
    msg(
        "error.rate_comments.message",
        "この 1 時間に、この提案へ何度もコメントしています。",
    ),
    msg(
        "error.rate_comments.hint",
        "しばらく待ってからもう一度お試しください。待ち時間は Retry-After ヘッダーにあります。",
    ),
    msg(
        "error.rate_decisions.message",
        "この 1 時間に、この提案へ何度も判断を記録しています。",
    ),
    msg(
        "error.rate_decisions.hint",
        "しばらく待ってからもう一度お試しください。待ち時間は Retry-After ヘッダーにあります。",
    ),
    msg(
        "error.invalid_request.comment_hint",
        "フィールドの一覧は API ドキュメントにあるので、reason に示されたフィールドを直してから、コメントをもう一度送信してください。",
    ),
    msg(
        "error.invalid_request.decision_hint",
        "フィールドの一覧は API ドキュメントにあるので、reason に示されたフィールドを直してから、判断をもう一度送信してください。",
    ),
    msg(
        "error.invalid_request.hide_hint",
        "500 文字までの一、二文の理由を付けて、もう一度送信してください。",
    ),
    msg("review.line.awaiting", "レビュー待ち"),
    msg("review.line.approved", "{count} 人が承認しました"),
    msg("review.line.changes", "{count} 人が変更を依頼しています"),
    msg(
        "review.means.awaiting",
        "リビジョン {revision} に判断を出したレビュアーはまだいません。レビューは何もマージしません。",
    ),
    msg(
        "review.means.approved",
        "ここでの承認は、レビュアーがリビジョン {revision} に満足していると述べたという意味です。何もマージしません。",
    ),
    msg(
        "review.means.changes",
        "レビュアーがリビジョン {revision} への変更を依頼しました。これは意見であり、マージも妨げも行いません。",
    ),
    msg(
        "review.earlier_note",
        "以前のリビジョンで承認されました。リビジョン {revision} はまだレビューされていません。",
    ),
    msg("review.word.approved", "承認済み"),
    msg("review.word.changes", "変更依頼あり"),
    msg("review.heading", "レビュー"),
    msg("review.decisions.heading", "リビジョン {revision} への判断"),
    msg(
        "review.decisions.none",
        "このリビジョンに判断を出した人はまだいません。",
    ),
    msg(
        "review.decision.approve",
        "{who} さんがリビジョン {revision} を承認しました",
    ),
    msg(
        "review.decision.request_changes",
        "{who} さんがリビジョン {revision} への変更を依頼しました",
    ),
    msg("review.decision.note", "メモ：{note}"),
    msg("review.earlier.heading", "以前のリビジョンへの判断"),
    msg(
        "review.earlier.explain",
        "これらは、その後置き換えられたリビジョンに対する判断です。履歴であり、現在は何にも数えられません。",
    ),
    msg("comments.heading", "コメント"),
    msg("comments.none", "コメントはまだありません。"),
    msg("comments.more", "さらにコメントを表示"),
    msg("comments.count", "コメント {count} 件"),
    msg(
        "comment.meta",
        "{who} · {when} · リビジョン {revision} の時点",
    ),
    msg("comment.anchor.patch", "パッチ {patch} について"),
    msg("comment.anchor.path", "{path} について"),
    msg("comment.anchor.lines", "{path} の {from}–{to} 行目について"),
    msg("comment.anchor.line", "{path} の {line} 行目について"),
    msg("comment.hidden", "メンテナーが非表示にしました：{reason}"),
    msg(
        "comment.hidden_admin",
        "{who} さんが {when} に非表示にしました：{reason}。あなたはこのリポジトリの管理者なので読めます。",
    ),
    msg("comment.reply.label", "返信"),
    msg("comment.reply.button", "返信する"),
    msg("comment.form.heading", "コメントを追加"),
    msg("comment.form.body", "コメント（Markdown を使えます）"),
    msg(
        "comment.form.patch",
        "パッチについて（任意）：その ID の最初の数文字",
    ),
    msg("comment.form.path", "ファイルについて（任意）：そのパス"),
    msg("comment.form.from", "開始行（任意）"),
    msg("comment.form.to", "終了行（任意）"),
    msg("comment.form.button", "コメントする"),
    msg(
        "comment.form.consequence",
        "このリポジトリを読める全員にコメントが見えます。あとから編集や削除はできず、メンテナーが非表示にすることはできます。",
    ),
    msg("comment.refused.heading", "コメントは追加されませんでした"),
    msg("comment.refusal.body_empty", "コメントには本文が必要です。"),
    msg(
        "comment.refusal.body_too_long",
        "コメントは 16 KiB までです。",
    ),
    msg(
        "comment.refusal.patch",
        "この提案のパッチで、その文字で始まるものがないか、複数あります。",
    ),
    msg(
        "comment.refusal.path",
        "パスは 1〜1,024 文字で、制御文字は使えません。",
    ),
    msg(
        "comment.refusal.lines_need_a_path",
        "行を指定するには、ファイルのパスも必要です。",
    ),
    msg(
        "comment.refusal.lines_incomplete",
        "終了行だけでなく開始行も指定してください。",
    ),
    msg("comment.refusal.line_zero", "行番号は 1 から始まります。"),
    msg(
        "comment.refusal.line_order",
        "開始行は終了行より後にできません。",
    ),
    msg(
        "comment.refusal.parent",
        "そこには返信を追加できません。スレッドは 1 階層までです。",
    ),
    msg("decision.form.heading", "リビジョン {revision} への判断"),
    msg("decision.form.approve", "承認する"),
    msg("decision.form.request_changes", "変更を依頼する"),
    msg("decision.form.note", "メモ（任意）"),
    msg("decision.form.button", "判断を記録する"),
    msg(
        "decision.form.consequence",
        "これはリビジョン {revision} だけに記録され、このリポジトリを読める全員に見えます。この判断は、提案が開いている間は変えられます。あとのリビジョンには引き継がれず、何もマージしません。",
    ),
    msg(
        "decision.form.self",
        "あなたはこの提案の一部を送ったため、承認はできません。変更の依頼はできます。",
    ),
    msg("decision.refused.heading", "判断は記録されませんでした"),
    msg("decision.refusal.note_too_long", "メモは 2 KiB までです。"),
    msg("hide.form.reason", "非表示にする理由"),
    msg("hide.form.button", "コメントを非表示にする"),
    msg(
        "hide.form.consequence",
        "コメントは記録に残ります。本文が見えるのはこのリポジトリの管理者だけで、ほかの人には理由が表示されます。あとで再表示できます。",
    ),
    msg("unhide.form.button", "コメントを再表示する"),
    msg(
        "unhide.form.consequence",
        "このリポジトリを読める全員に本文が再び見えます。",
    ),
    msg("hide.refused.heading", "コメントは変わりませんでした"),
    msg(
        "hide.refusal.reason",
        "非表示にするには、500 文字までの理由が必要です。",
    ),
    msg("proposals.col.review", "レビュー"),
    // repository creation (RFC 004 part 7)
    msg(
        "error.repository_quota.message",
        "{owner} にはすでにリポジトリが {used} 件あり、このサーバーが認める上限（{limit} 件）に達しています。",
    ),
    msg(
        "error.repository_quota.hint",
        "上限を上げるには、このサーバーを運用している人に頼んでください。",
    ),
    msg(
        "error.name_taken.message",
        "その名前のリポジトリはすでにあります。",
    ),
    msg(
        "error.name_taken.hint",
        "別の名前を選んで、もう一度送ってください。",
    ),
    msg(
        "error.stage_limit.message",
        "確定待ちのアーカイブがすでに {limit} 件あります。",
    ),
    msg(
        "error.stage_limit.hint",
        "どれかを確定するか、待ってください。確定されないアーカイブは 1 日たつと削除されます。",
    ),
    msg(
        "error.stage_budget.message",
        "このサーバーは、保持できる数のアーカイブをすでに預かっています。",
    ),
    msg(
        "error.stage_budget.hint",
        "しばらくしてからもう一度試し、続く場合はサーバーを運用している人に伝えてください。",
    ),
    msg(
        "error.archive_refused.message",
        "アーカイブは受け付けられませんでした。",
    ),
    msg(
        "error.archive_refused.hint",
        "prikk の説明は detail に入っているので、メンテナーに新しいアーカイブ（prikk archive export）を作ってもらい、それを送ってください。",
    ),
    msg(
        "error.archive_keys.message",
        "アーカイブに載っているメンテナーの鍵を、このサーバーは読み取れません。",
    ),
    msg(
        "error.archive_keys.hint",
        "最新の prikk で作ったアーカイブを、メンテナーに頼んでください。",
    ),
    msg(
        "error.stage_expired.message",
        "そのアーカイブは 1 日だけ保管され、その期間が過ぎました。",
    ),
    msg(
        "error.stage_expired.hint",
        "アーカイブをもう一度送ってください。",
    ),
    msg(
        "error.stage_in_use.message",
        "そのアーカイブは、いまリポジトリの作成に使われています。",
    ),
    msg(
        "error.stage_in_use.hint",
        "それが終わるまで待ってから、結果を見てください。",
    ),
    msg(
        "error.adopt_mismatch.message",
        "鍵 {key} について入力したフィンガープリントは、アーカイブに載っているものと一致しません。",
    ),
    msg(
        "error.adopt_mismatch.hint",
        "何も作成されていないので、メンテナーから聞いたものともう一度見比べてください。",
    ),
    msg(
        "error.adopt_malformed.message",
        "鍵 {key} について入力したものは、フィンガープリントではありません。",
    ),
    msg(
        "error.adopt_malformed.hint",
        "フィンガープリントは a〜f と 0〜9 の 64 文字（空白は可）で、何も作成されていません。",
    ),
    msg(
        "error.adopt_unaddressed.message",
        "{keys} をどうするかが指定されていません。",
    ),
    msg(
        "error.adopt_unaddressed.hint",
        "何も作成されていないので、信頼する鍵ごとにフィンガープリントを入力し、信頼しない鍵はその旨を選んでください。",
    ),
    msg(
        "error.adopt_other.message",
        "入力された内容が、このアーカイブの鍵と合いません。",
    ),
    msg(
        "error.adopt_other.hint",
        "何も作成されていないので、アーカイブの鍵は 1 つずつ、フィンガープリントを付けるか、除外する鍵として指定してください。",
    ),
    msg(
        "error.creation_verify.message",
        "このアーカイブから作られるリポジトリは prikk の検証に通らないため、作成されませんでした。",
    ),
    msg(
        "error.creation_verify.hint",
        "何も保存されておらず、失敗した内容は conditions にあるので、メンテナーに新しいアーカイブを頼んでください。",
    ),
    msg(
        "error.forbidden_create.message",
        "そこにはリポジトリを作成できません。",
    ),
    msg(
        "error.forbidden_create.hint",
        "自分のユーザー名か、所属している組織の名前を使ってください。",
    ),
    msg(
        "error.forbidden_create_signin.message",
        "リポジトリを作成するにはアカウントが必要です。",
    ),
    msg(
        "error.forbidden_create_signin.hint",
        "リポジトリを所有するアカウントのトークンを付けて送ってください。",
    ),
    msg(
        "error.invalid_request.name_hint",
        "この応答の field と reason に、拒否された値と理由が示されています。API ドキュメントの項目の通りに直して、もう一度送ってください。",
    ),
    msg(
        "error.import_off.message",
        "このサーバーは、アーカイブからのリポジトリ作成を設定していません。",
    ),
    msg(
        "error.import_off.hint",
        "アーティファクト用ディレクトリを設定するよう、サーバーを運用している人に頼んでください。",
    ),
    msg(
        "error.rate_creations.message",
        "この 1 時間に開始した新しいリポジトリが多すぎます。",
    ),
    msg(
        "error.rate_creations.hint",
        "Retry-After ヘッダーの秒数のあとにもう一度送ってください。",
    ),
    msg(
        "error.create_timed_out.message",
        "リポジトリの作成がこのサーバーの許す時間を超えたため、何も保存されていません。",
    ),
    msg(
        "error.create_timed_out.hint",
        "もう一度試し、続く場合はアーカイブがこのサーバーには大きすぎるので、運用している人に伝えてください。",
    ),
    msg(
        "error.create_too_big.message",
        "このアーカイブは、サーバーが 1 件に許すメモリを超えて必要とします。",
    ),
    msg(
        "error.create_too_big.hint",
        "何も保存されていないので、上限を上げられるか、サーバーを運用している人に尋ねてください。",
    ),
    msg(
        "error.import_too_large.message",
        "そのアーカイブは、このサーバーが受け付ける大きさを超えています。",
    ),
    msg(
        "error.import_too_large.hint",
        "もっと小さいアーカイブをメンテナーに頼むか、上限を上げるようサーバーを運用している人に頼んでください。",
    ),
    msg(
        "error.delivery_too_large.message",
        "そのアーカイブは、このサーバーが受け付ける大きさを超えています。",
    ),
    msg(
        "error.delivery_too_large.hint",
        "引き渡せるリポジトリは {limit} MiB までです。もっと小さいリポジトリを引き渡すか、上限を上げるようこのサーバーを運用している人に頼んでください。",
    ),
    msg(
        "error.import_length.message",
        "リクエストの長さが示されていません。",
    ),
    msg(
        "error.import_length.hint",
        "Content-Length ヘッダーを付けてアーカイブを送ってください。",
    ),
    msg(
        "error.invalid_request.import_framing_hint",
        "本文は PLANETER-IMPORT-1 で始め、続けてメタデータの長さ、メタデータ、アーカイブの順に、API ドキュメントの通りに送ってください。",
    ),
    msg("title.new", "新しいリポジトリ · {product}"),
    msg("new.heading", "新しいリポジトリ"),
    msg(
        "new.intro",
        "始め方を選んでください。選んだ方のボタンを押すまで、何も作成されません。",
    ),
    msg("new.owner", "所有者"),
    msg(
        "new.owner_hint",
        "自分のユーザー名か、所属している組織の名前です。",
    ),
    msg("new.name", "リポジトリ名"),
    msg(
        "new.name_hint",
        "小文字、数字、ドット、ハイフン、アンダースコアが使え、64 文字までです。",
    ),
    msg("new.visibility", "読める人"),
    msg(
        "new.visibility.public",
        "誰でも（ログインの有無を問わない）",
    ),
    msg("new.visibility.internal", "ログインしている人なら誰でも"),
    msg("new.visibility.private", "アクセスを与えた人だけ"),
    msg("new.proposals", "提案を送れる人"),
    msg("new.proposals.members", "書き込み権限を与えた人"),
    msg("new.proposals.any", "読めるログイン済みの人なら誰でも"),
    msg(
        "new.proposals.any_explain",
        "知らない人からの貢献を受け付ける公開プロジェクトでは、後者を選びます。その人たちの作業はレビュー待ちで保持され、それだけでブランチが変わることはありません。",
    ),
    msg("new.empty.heading", "空のリポジトリ"),
    msg("new.empty.explain", "履歴なしで始めます。"),
    msg("new.empty.button", "空のリポジトリを作成する"),
    msg(
        "new.empty.consequence",
        "リポジトリをいま作成します。あなたがそのリポジトリの管理者になり、上で選んだ人が読めるようになります。",
    ),
    msg(
        "new.import.heading",
        "メンテナーから受け取ったアーカイブから",
    ),
    msg(
        "new.import.explain",
        "メンテナーは、リポジトリ全体を 1 つのアーカイブファイルとして書き出せます（prikk archive export）。ここにアップロードすると、その履歴をすべて持った状態で始められます。作成の前に、中身を確認できます。",
    ),
    msg("new.import.file", "アーカイブファイル"),
    msg(
        "new.import.max",
        "このサーバーが受け付けるアーカイブは {size} までです。",
    ),
    msg("new.import.button", "アーカイブをアップロードして確認する"),
    msg(
        "new.import.consequence",
        "アーカイブを確認し、1 日だけ保管します。リポジトリはまだ作成されず、中身は何も信頼されません。",
    ),
    msg(
        "new.import.off",
        "このサーバーは、アーカイブからのリポジトリ作成を設定していません。",
    ),
    msg("new.refused.heading", "リポジトリは作成されませんでした"),
    msg("new.error.owner", "その所有者名は使えません。"),
    msg("new.error.name.empty", "リポジトリに名前を付けてください。"),
    msg("new.error.name.too_long", "名前は 64 文字までです。"),
    msg(
        "new.error.name.uppercase",
        "大文字と小文字だけが違う 2 つの名前を避けるため、小文字だけを使ってください。",
    ),
    msg(
        "new.error.name.bad_character",
        "名前に使えるのは、小文字、数字、ドット、ハイフン、アンダースコアだけです。",
    ),
    msg(
        "new.error.name.bad_start",
        "名前は文字か数字で始めてください。",
    ),
    msg(
        "new.error.name.dotted",
        "名前にドットを 2 つ続けたり、ドットや .git で終えたりはできません。",
    ),
    msg(
        "new.error.name.reserved",
        "その名前はこのサーバー自身が使っています。別の名前にしてください。",
    ),
    msg("new.error.visibility", "読める人を選んでください。"),
    msg("new.error.proposals", "提案を送れる人を選んでください。"),
    msg(
        "new.error.file",
        "アップロードするアーカイブファイルを選んでください。",
    ),
    msg(
        "new.error.upload",
        "アップロードを、ファイル付きのフォームとして読み取れませんでした。何も保存されていません。",
    ),
    msg("title.stage", "アーカイブの確認 · {product}"),
    msg("stage.heading", "{repo} 用のアーカイブ"),
    msg(
        "stage.intro",
        "prikk はアーカイブの内部に問題を見つけませんでした。ただし、それで信頼できるとは限りません。まだ何も作成されていません。",
    ),
    msg("stage.size", "サイズ：{size}"),
    msg("stage.digest", "SHA-256：{digest}"),
    msg("stage.expires", "{when} まで保管し、その後削除します。"),
    msg("stage.objects", "全部で {count} 件の項目を含みます。"),
    msg("stage.refs.heading", "アーカイブ内のブランチ"),
    msg(
        "stage.ref.published",
        "{ref}：署名した鍵を信頼すれば、新しいリポジトリのブランチになります",
    ),
    msg(
        "stage.ref.kept",
        "{ref}：ブランチにはならず、受信したコピーとして保持されます（{reason}）",
    ),
    msg(
        "stage.ref.received",
        "{ref}：メンテナーが別の場所から受け取ったコピーで、常に受信したコピーとして保持されます",
    ),
    msg("stage.ref.more", "ほか {count} 件のブランチ"),
    msg(
        "stage.verifies.yes",
        "下の鍵をすべて信頼すれば、新しいリポジトリは prikk のすべての検証に通ります。",
    ),
    msg(
        "stage.verifies.no",
        "下の鍵をすべて信頼しても、新しいリポジトリは prikk の検証に通りません（{problems}）。鍵を 1 つも信頼せずに作成することはできます。",
    ),
    msg("stage.keys.heading", "アーカイブ内のメンテナーの鍵"),
    msg(
        "stage.keys.explain",
        "アーカイブは、これらをメンテナーの鍵として載せています。アーカイブには誰でも何でも書けるので、その一覧は何の証明にもなりません。このアーカイブとは別の方法でメンテナーに鍵のフィンガープリントを尋ね、下に入力してください。信頼されるのは、入力した鍵だけです。",
    ),
    msg(
        "stage.keys.none",
        "アーカイブにはメンテナーの鍵が載っていないため、信頼するものがありません。何も信頼せずにリポジトリを作成します。",
    ),
    msg("stage.key.id", "鍵 {key}"),
    msg("stage.key.fingerprint", "照合するフィンガープリント："),
    msg(
        "stage.key.input",
        "この鍵を信頼するには、フィンガープリントを入力してください",
    ),
    msg("stage.key.skip", "この鍵を信頼しない"),
    msg(
        "stage.confirm.button",
        "リポジトリを作成し、入力した鍵を信頼する",
    ),
    msg(
        "stage.confirm.consequence",
        "{repo} を作成し、アーカイブを取り込み、フィンガープリントを入力した鍵だけを信頼します。途中で問題が起きた場合は、何も保存されません。",
    ),
    msg(
        "stage.none.button",
        "鍵を 1 つも信頼せずにリポジトリを作成する",
    ),
    msg(
        "stage.none.consequence",
        "{repo} を作成し、すべてを受信したコピーとして保持します。鍵を信頼するまで何も検証・信頼されず、このバージョンでは後から鍵を信頼することもできません。",
    ),
    msg("stage.back", "最初からやり直す"),
    msg("stage.refused.heading", "リポジトリは作成されませんでした"),
    msg("title.created", "{repo} を作成しました · {product}"),
    msg("created.heading", "リポジトリ {repo} を作成しました"),
    msg(
        "created.branch.published",
        "{ref}：リポジトリのブランチです",
    ),
    msg(
        "created.branch.kept",
        "{ref}：{landed} という名前の受信したコピーとして保持されています（{reason}）",
    ),
    msg(
        "created.branch.kept_plain",
        "{ref}：{landed} という名前の受信したコピーとして保持されています",
    ),
    msg(
        "created.verified",
        "リポジトリは prikk のすべての検証に通りました。",
    ),
    msg(
        "created.unverified",
        "信頼しなかった鍵で署名されたブランチがあるため、リポジトリは prikk のすべての検証には通りません。鍵を信頼するまで何も検証・信頼されず、このバージョンでは後から鍵を信頼することもできません。",
    ),
    msg("created.trusted", "信頼した鍵：{keys}。"),
    msg("created.none_trusted", "信頼した鍵はありません。"),
    msg("created.open", "{repo} を開く"),
    msg(
        "created.next",
        "次は、リポジトリを開いて中身を見てください。",
    ),
    // repository creation (RFC 004 part 7)
    msg(
        "new.signin",
        "リポジトリを作成するには、ログインしてください。",
    ),
    msg("new.nav", "新しいリポジトリ"),
    msg("stage.prikk", "prikk の説明：{detail}"),
    msg(
        "stage.error.both",
        "鍵 {key} について、フィンガープリントを入力し、同時に信頼しないことも選んでいます。どちらか一方にしてください。",
    ),
    msg("stage.error.verify", "失敗したチェック：{conditions}。"),
    msg(
        "stage.gone",
        "そのアーカイブはもうありません。すでに使われたか、保管期間が過ぎた可能性があります。",
    ),
    msg("stage.gone.link", "リポジトリを作成する"),
    // ---- what no route handles, and a request that is too large (japanese-final-handoff T2)
    msg(
        "error.payload_too_large.request_message",
        "そのリクエストは、このサーバーが受け付ける大きさを超えています。",
    ),
    msg(
        "error.payload_too_large.request_hint",
        "もっと小さいリクエストを送るか、このサーバーを運用している人に尋ねてください。",
    ),
    msg(
        "page.method_not_allowed.title",
        "この操作はここでは使えません · {product}",
    ),
    msg(
        "page.method_not_allowed.heading",
        "この操作はここでは使えません",
    ),
    msg(
        "page.method_not_allowed.body",
        "このアドレスは、その種類のリクエストを受け付けないため、何も行われていません。前のページに戻って、ページのリンクやボタンから操作してください。",
    ),
    msg(
        "page.payload_too_large.title",
        "リクエストが大きすぎます · {product}",
    ),
    msg("page.payload_too_large.heading", "リクエストが大きすぎます"),
    msg(
        "page.payload_too_large.body",
        "送られた内容が、このサーバーが一度に受け付ける大きさを超えているため、何も行われていません。小さくして送り直すか、このサーバーを運用している人に尋ねてください。",
    ),
    msg(
        "error.delivery_busy.message",
        "このリポジトリへの別の引き渡しがすでに進行中か、リポジトリがほかの書き込みで使用中です。",
    ),
    msg(
        "error.delivery_busy.hint",
        "しばらく待ってから、もう一度お試しください。確定待ちの引き渡しがある場合は、先に確定するか取り消してください。",
    ),
    msg(
        "error.delivery_not_confirmable.message",
        "この引き渡しは、今は確定できません。",
    ),
    msg(
        "error.delivery_not_confirmable.hint",
        "引き渡しの状態を確認してください。確定できるのは、確認を終えて受け入れられた引き渡しだけで、確定は 1 回だけです。",
    ),
    msg(
        "error.forbidden_deliver.message",
        "このリポジトリは読めますが、引き渡すことはできません。",
    ),
    msg(
        "error.forbidden_deliver.hint",
        "引き渡せるのは、リポジトリのメンテナーだけです。メンテナーに依頼するか、引き渡しが許可されたトークンを使ってください。",
    ),
    msg(
        "delivery.state.staged.message",
        "アーカイブを確認しています。",
    ),
    msg(
        "delivery.state.staged.hint",
        "しばらく待ってから、この引き渡しをもう一度確認してください。",
    ),
    msg(
        "delivery.accepted.message",
        "引き渡すと、このリポジトリのブランチ {ref} があなたのブランチに置き換わります。置き換わるのは、あなたのブランチが今のブランチの続きである場合だけです。このサーバーは何にも署名しません。",
    ),
    msg(
        "delivery.accepted.hint",
        "実行するには確定してください。やめるには取り消してください。確定されない引き渡しは、1 日たつと削除されます。",
    ),
    msg(
        "delivery.state.confirmed.message",
        "引き渡しを実行しています。",
    ),
    msg(
        "delivery.state.confirmed.hint",
        "しばらく待ってから、この引き渡しをもう一度確認してください。もう一度送信しないでください。",
    ),
    msg(
        "delivery.state.exchanged.message",
        "新しいブランチに切り替わりました。開いている提案を、新しいブランチに戻すところです。",
    ),
    msg(
        "delivery.state.exchanged.hint",
        "しばらく待ってから、この引き渡しをもう一度確認してください。",
    ),
    msg(
        "delivery.state.reaccepting.message",
        "開いている提案を、新しいブランチに戻しています。",
    ),
    msg(
        "delivery.state.reaccepting.hint",
        "しばらく待ってから、この引き渡しをもう一度確認してください。",
    ),
    msg(
        "delivery.state.done.message",
        "引き渡しが完了しました。このリポジトリのブランチは、あなたのブランチになりました。",
    ),
    msg(
        "delivery.state.done.hint",
        "開いていた提案は、そのまま開いています。あなたのブランチに作業が入っている提案は、反映済みと表示されます。",
    ),
    msg(
        "delivery.state.failed.message",
        "引き渡しは完了しませんでした。",
    ),
    msg(
        "delivery.state.failed.hint",
        "理由を確認して、もう一度引き渡してください。",
    ),
    msg(
        "delivery.state.abandoned.message",
        "この引き渡しは取りやめになりました。",
    ),
    msg(
        "delivery.state.abandoned.hint",
        "準備ができたら、もう一度引き渡してください。",
    ),
    msg(
        "delivery.state.expired.message",
        "この引き渡しは 1 日以上待たれたため、削除されました。",
    ),
    msg(
        "delivery.state.expired.hint",
        "もう一度引き渡してください。",
    ),
    msg(
        "delivery.needs_recovery.message",
        "この引き渡しの実行中に、サーバーが停止しました。",
    ),
    msg(
        "delivery.needs_recovery.hint",
        "サーバーがリポジトリを確認すると、自動的に完了します。完了しない場合は、リポジトリの管理者に、リポジトリの復旧の実行を依頼してください。",
    ),
    msg(
        "delivery.reason.no_main.message",
        "アーカイブに、{ref} というブランチがありません。",
    ),
    msg(
        "delivery.reason.no_main.hint",
        "プロジェクトのブランチがあるリポジトリのアーカイブを引き渡してください。",
    ),
    msg(
        "delivery.reason.unadopted_key.message",
        "履歴の一部が、このリポジトリが信頼していない鍵で署名されています。",
    ),
    msg(
        "delivery.reason.unadopted_key.hint",
        "引き渡せるのは、ここに一覧がある鍵だけです。",
    ),
    msg(
        "delivery.reason.not_a_successor.message",
        "アーカイブは、このリポジトリのブランチ {ref} の続きになっていません。",
    ),
    msg(
        "delivery.reason.not_a_successor.hint",
        "このリポジトリの内容を自分のリポジトリに取り込んでから、もう一度引き渡してください。",
    ),
    msg(
        "delivery.reason.older_or_diverged.message",
        "あなたのブランチは、このリポジトリのブランチの続きになっていません。",
    ),
    msg(
        "delivery.reason.older_or_diverged.hint",
        "プロジェクトのブランチを自分のリポジトリに取り込んでから、もう一度引き渡してください。",
    ),
    msg(
        "delivery.reason.unrelated.message",
        "アーカイブの履歴には、このリポジトリの履歴と共通するものがありません。",
    ),
    msg(
        "delivery.reason.unrelated.hint",
        "このリポジトリから作ったリポジトリのアーカイブを引き渡してください。別の履歴で置き換えることはできません。",
    ),
    msg(
        "delivery.reason.branch_missing.message",
        "このリポジトリにあるブランチ（{ref}）が、引き渡しにありません。",
    ),
    msg(
        "delivery.reason.branch_missing.hint",
        "引き渡しでブランチを追加することはできますが、削除することはできません。",
    ),
    msg(
        "delivery.reason.nothing_to_deliver.message",
        "アーカイブには、このリポジトリに加わる変更がありません。",
    ),
    msg(
        "delivery.reason.nothing_to_deliver.hint",
        "引き渡すものは、新しくありません。",
    ),
    msg(
        "delivery.reason.damaged_archive.message",
        "アーカイブを読み込めないか、チェックに通りませんでした。",
    ),
    msg(
        "delivery.reason.damaged_archive.hint",
        "prikk でアーカイブを作り直して、もう一度送信してください。",
    ),
    msg(
        "delivery.reason.moved.message",
        "引き渡しを待っている間に、このリポジトリが変わりました。",
    ),
    msg(
        "delivery.reason.moved.hint",
        "プロジェクトのブランチを自分のリポジトリに取り込んでから、もう一度引き渡してください。",
    ),
    msg(
        "delivery.reason.exchange_failed.message",
        "引き渡しを反映できず、何も変わっていません。",
    ),
    msg(
        "delivery.reason.exchange_failed.hint",
        "このサーバーを運用している人に伝えてから、もう一度お試しください。",
    ),
    msg(
        "delivery.reason.verify_failed.message",
        "ブランチは置き換わりましたが、リポジトリがチェックに通らなくなりました。",
    ),
    msg(
        "delivery.reason.verify_failed.hint",
        "リポジトリは、リポジトリの管理者の対応を待っています。確認が済むまで、もう一度引き渡さないでください。",
    ),
    msg(
        "delivery.reason.publication_trust.message",
        "ブランチは置き換わりましたが、変更の 1 つが、このリポジトリが信頼していない鍵で署名されています。",
    ),
    msg(
        "delivery.reason.publication_trust.hint",
        "リポジトリは、リポジトリの管理者の対応を待っています。確認が済むまで、もう一度引き渡さないでください。",
    ),
    msg(
        "delivery.reason.timed_out.message",
        "アーカイブの確認に、このサーバーが許す時間より長くかかりました。",
    ),
    msg(
        "delivery.reason.timed_out.hint",
        "しばらくしてからもう一度お試しください。または、小さいアーカイブにしてください。",
    ),
    msg(
        "delivery.reason.ceiling.message",
        "アーカイブの確認に、このサーバーが許すより多くのメモリが必要です。",
    ),
    msg(
        "delivery.reason.ceiling.hint",
        "小さいアーカイブにするか、このサーバーを運用している人に相談してください。",
    ),
    msg(
        "delivery.reason.unavailable.message",
        "今はアーカイブを確認できません。",
    ),
    msg(
        "delivery.reason.unavailable.hint",
        "しばらくしてから、もう一度お試しください。",
    ),
    msg(
        "delivery.reason.repository_gone.message",
        "このリポジトリはもうありません。",
    ),
    msg(
        "delivery.reason.repository_gone.hint",
        "何も変わっていません。",
    ),
    msg(
        "repo.delivered_by",
        "{name} が {date} に引き渡しました。このリポジトリが信頼する鍵で署名され、prikk がチェックしています。",
    ),
    msg(
        "repo.project_branch",
        "プロジェクトのブランチは {ref} です。",
    ),
    msg("repo.trusted_keys", "このリポジトリが信頼する鍵"),
    msg("repo.keys.count", "（{total}）"),
    msg(
        "repo.keys.none",
        "このリポジトリが信頼している鍵はありません。",
    ),
    msg(
        "repo.keys.unavailable",
        "このリポジトリが信頼する鍵を、今は読めません。",
    ),
    msg("repo.keys.source.creation", "リポジトリの作成時の鍵です。"),
    msg(
        "repo.keys.source.creation_recorded",
        "リポジトリの作成時の鍵です（{date} に記録）。",
    ),
    msg(
        "repo.keys.source.operator",
        "このサーバーの運用者が {date} に追加しました: {reason}",
    ),
    msg(
        "repo.keys.source.delivery",
        "{date} に、引き渡しで追加されました。",
    ),
    msg(
        "repo.keys.source.not_recorded",
        "どこから来た鍵か、記録がありません。",
    ),
    msg(
        "stage.key.slots",
        "このリポジトリが信頼できる鍵は、最大 {max} 個です。鍵はまだ取り除けないため、追加した鍵は、リポジトリが続くかぎり 1 つ分の枠を使います。{carried} 個すべてを信頼すると残りは {left} 個で、一部だけにすると、残りはそれより多くなります。",
    ),
    msg(
        "stage.key.slots_one",
        "このリポジトリが信頼できる鍵は、最大 {max} 個です。鍵はまだ取り除けないため、追加した鍵は、リポジトリが続くかぎり 1 つ分の枠を使います。この鍵を信頼すると、残りは {left} 個です。",
    ),
    msg(
        "operator.help.purpose",
        "このサーバーを運用している人のためのコマンドです。サーバーのデータが必要で、ウェブからは実行できません。先にサーバーを止めてください。",
    ),
    msg(
        "operator.help.usage",
        "使い方: planeter operator key add OWNER/NAME --key-id ID --public-key HEX --reason TEXT [--lang en|ja]\n       planeter operator --help\n\nkey add        公開鍵をもう 1 つ、1 つのリポジトリのメンテナーの鍵にします。そのリポジトリは、この鍵で署名された引き渡しを受け付けるようになります。鍵を加えるだけです\n--key-id       鍵の ID です。ed25519- に続く 16 文字で、0〜9 と小文字の a〜f を使います\n--public-key   公開鍵です。64 文字で、0〜9 と小文字の a〜f を使います\n--reason       鍵を加える理由です。1〜200 文字で、リポジトリのページに表示されます\n--lang         これらのメッセージの言語です（省略すると、LC_ALL、LC_MESSAGES、LANG のうち指定されているものの言語、なければ英語です）",
    ),
    msg(
        "operator.help.exit_codes",
        "終了コード: 0 完了、1 拒否（リポジトリまたは鍵）、2 コマンドラインの誤り、3 サーバーが動いている、またはロックを取れない、4 変更のあとで prikk または記録が失敗、5 ローカルのファイルまたはデータベース",
    ),
    msg(
        "operator.error.usage",
        "このコマンドラインは解釈できません。planeter operator --help で、指定できるものを確認してください。",
    ),
    msg(
        "operator.error.repo_form",
        "リポジトリは OWNER/NAME の形で指定してください。確認して、もう一度実行してください。",
    ),
    msg(
        "operator.error.reason_missing",
        "理由が必要です。鍵を加える理由を、1〜{max} 文字で書いてください。",
    ),
    msg(
        "operator.error.reason_long",
        "理由が {length} 文字あり、{max} 文字までです。短くして、もう一度実行してください。",
    ),
    msg(
        "operator.error.reason_control",
        "理由に、改行など、表示できない文字が含まれています。1 行の普通の文章にして、もう一度実行してください。",
    ),
    msg(
        "operator.error.key_shape",
        "公開鍵は 64 文字で、各文字は 0〜9 か小文字の a〜f にしてください。確認して、もう一度実行してください。",
    ),
    msg(
        "operator.error.key_id_shape",
        "鍵の ID は、ed25519- に続けて 16 文字にし、各文字は 0〜9 か小文字の a〜f にしてください。確認して、もう一度実行してください。",
    ),
    msg(
        "operator.error.key_id_mismatch",
        "鍵の ID {given} は、この公開鍵のものではありません。この公開鍵から prikk が導く ID は {derived} です。その ID を使うか、公開鍵が意図したものか確認してください。",
    ),
    msg(
        "operator.error.running",
        "サーバーが動いています。サーバーを止めてから、このコマンドをもう一度実行し、そのあとでサーバーを起動してください。",
    ),
    msg(
        "operator.error.lock",
        "サーバーのデータベースのロックを取れません: {why}。サーバーが動いていないなら、そこに書かれているファイルを確認してください。",
    ),
    msg(
        "operator.error.no_database",
        "{path} にデータベースがありません。PLANETER_DB にサーバーのデータベースを指定して、もう一度実行してください。",
    ),
    msg(
        "operator.error.database",
        "データベースを開けません: {why}。何も変わっていません。",
    ),
    msg(
        "operator.error.unknown_repo",
        "リポジトリ {repo} はありません。所有者と名前を確認してください。",
    ),
    msg(
        "operator.error.needs_recovery",
        "先にサーバーを起動して、復旧が終わるのを待ってください。",
    ),
    msg(
        "operator.error.already_trusted",
        "この鍵は、すでに信頼されています。",
    ),
    msg(
        "operator.error.bound",
        "このリポジトリは、planeter が対応する最大数の鍵（{max} 個）をすでに信頼しています。鍵はまだ取り除けないため、これ以上は加えられません。",
    ),
    msg(
        "operator.error.prikk",
        "prikk を実行できません: {why}。何も変わっていません。",
    ),
    msg(
        "operator.error.add",
        "prikk が鍵を加えませんでした: {why}。サーバーを起動する前に、prikk でリポジトリを確認してください。",
    ),
    msg(
        "operator.error.verify",
        "鍵は加わりましたが、リポジトリがチェックに通らなくなりました。鍵を取り除くことはできません。リポジトリには復旧が必要という印を付けました。サーバーを起動して、リポジトリの管理者に確認を依頼してください。",
    ),
    msg(
        "operator.error.record",
        "鍵は信頼されていますが、その記録を書けませんでした。同じコマンドをもう一度実行して、記録してください。",
    ),
    msg(
        "operator.error.files",
        "コマンドに必要なファイルまたはディレクトリを読めません: {why}。何も変わっていません。",
    ),
    msg(
        "operator.done",
        "{fingerprint} は、{repo} で信頼されるようになりました。このサーバーの運用者が {date} に追加しました: {reason}",
    ),
    msg(
        "operator.done.recorded",
        "{fingerprint} は、{repo} で、すでに信頼されていました。記録がなかったため、今、書きました。このサーバーの運用者が {date} に追加しました: {reason}",
    ),
    msg("operator.start", "サーバーを起動してください。"),
    msg(
        "operator.token.help.summary",
        "token …        アカウントのアクセストークンを作り、一覧にし、取り消します（create, list, revoke）。これらは、動いているサーバーのそばで実行できます。詳しくは planeter operator token --help を実行してください。",
    ),
    msg(
        "operator.token.help.usage",
        "使い方: planeter operator token create HANDLE --name TEXT --scope read-only|write|full [--expires-days N] [--repos OWNER/NAME[,OWNER/NAME…]] [--out FILE]\n        planeter operator token list HANDLE\n        planeter operator token revoke HANDLE ID\n       （どれも [--lang en|ja] を付けられます）\n\ncreate         アカウントのトークンを作ります。表示されるのは 1 回だけで、保存されるのはそのハッシュだけです\n--name         トークンの用途。1〜60 文字。持ち主が自分のトークンを見分けるためのものです\n--scope        read-only: リポジトリを見て、クローンできます。write: さらに、作業を提案できますが、ブランチの引き渡しはできません。full: アカウントにできることはすべて、ブランチの引き渡しも含めてできます\n--expires-days トークンの有効日数。1〜365（省略すると 90）。期限のないトークンはありません\n--repos        トークンを、指定したリポジトリ（20 個まで）に限ります。指定しなければ、アカウントが届くすべてのリポジトリに届きます\n--out          トークンを表示せず、新しいファイル（モード 0600、上書きしません）に書きます。そのファイルを planeter-client --token-file に渡します\nlist           アカウントのトークンを新しい順に表示します: ID、名前、スコープ、リポジトリ、作成日、最後に使った日、期限。秘密は表示しません\nrevoke         指定した ID のトークンを、次のリクエストから受け付けなくします",
    ),
    msg(
        "operator.token.help.output",
        "トークン: 標準では、標準出力に 1 行で表示し、ほかのものは標準出力に出しません。そのため、TOKEN=$(planeter operator token create HANDLE --name TEXT --scope full) のように使えます。何のトークンか、いつまで有効かは、標準エラー出力に出します。--out を付けると、ファイルに書き、表示しません。",
    ),
    msg(
        "operator.token.error.usage",
        "このコマンドラインは解釈できません。planeter operator token --help を実行して、指定できるものを確認してください。",
    ),
    msg(
        "operator.token.error.account_disabled",
        "{handle} は無効になっているので、トークンを作れません。先に有効にしてください。",
    ),
    msg(
        "operator.token.error.name_missing",
        "名前が必要です。--name TEXT に、ほかのトークンと見分けられるように、1〜{max} 文字で書いてください。",
    ),
    msg(
        "operator.token.error.name_long",
        "名前は {length} 文字あり、{max} 文字までです。短くして、もう一度実行してください。",
    ),
    msg(
        "operator.token.error.name_unsafe",
        "名前に、改行など、表示できない文字が含まれています。普通の文章にして、もう一度実行してください。",
    ),
    msg(
        "operator.token.error.scope_missing",
        "スコープが必要です。--scope に read-only、write、full のどれかを指定してください。",
    ),
    msg(
        "operator.token.error.scope_unknown",
        "スコープは read-only、write、full のどれかにしてください。",
    ),
    msg(
        "operator.token.error.days",
        "--expires-days は、1〜{max} の整数の日数にしてください。",
    ),
    msg(
        "operator.token.error.repos_too_many",
        "トークンを限れるリポジトリは、{max} 個までです。",
    ),
    msg(
        "operator.token.error.repos_empty",
        "--repos には、OWNER/NAME を 1 つ以上、カンマで区切って指定してください。",
    ),
    msg(
        "operator.token.error.too_many_tokens",
        "{handle} には、期限が切れていないトークンがすでに {max} 個あります。先に planeter operator token revoke で 1 つ取り消してください。",
    ),
    msg(
        "operator.token.error.out_exists",
        "{file} はすでにあり、トークンのファイルは上書きしません。まだないファイルを指定してください。何も保存していません。",
    ),
    msg(
        "operator.token.error.out_failed",
        "トークンを {file} に書けませんでした: {why}。何も保存していません。",
    ),
    msg(
        "operator.token.error.no_token",
        "{handle} には、トークン {id} はありません。planeter operator token list {handle} で、トークンを確認してください。",
    ),
    msg(
        "operator.token.done.created",
        "{handle} のトークン（{name}）を作りました。ID は {id} です。",
    ),
    msg(
        "operator.token.done.once",
        "トークンが表示されるのは、この 1 回だけです。人に見られない場所に保管してください。もう一度表示することはできません。",
    ),
    msg("operator.token.done.expires", "{date} までです。"),
    msg(
        "operator.token.scope.read_only",
        "読み取り専用です。リポジトリを見て、クローンすることだけができます。",
    ),
    msg(
        "operator.token.scope.write",
        "読むことと、作業を提案することができますが、ブランチを引き渡すことはできません。",
    ),
    msg(
        "operator.token.scope.full",
        "このトークンは、アカウントにできることをすべて、ブランチの引き渡しも含めてできます。",
    ),
    msg(
        "operator.token.repos.all",
        "アカウントが届くすべてのリポジトリに届きます。",
    ),
    msg(
        "operator.token.repos.only",
        "次のリポジトリにだけ届きます: {repos}。",
    ),
    msg(
        "operator.token.done.out",
        "トークンを {file} に書きました（モード 0600）。このパスを planeter-client --token-file に渡してください。",
    ),
    msg(
        "operator.token.done.revoked",
        "{handle} のトークン {id}（{name}）を取り消しました。次のリクエストから受け付けられません。",
    ),
    msg(
        "operator.token.list.header",
        "ID | 名前 | スコープ | リポジトリ | 作成日 | 最後に使った日 | 期限",
    ),
    msg(
        "operator.token.list.none",
        "{handle} には、トークンがありません。",
    ),
    msg("operator.token.list.never", "なし"),
    msg("operator.token.list.all", "すべて"),
    msg("operator.token.list.read_only", "読み取り専用"),
    msg("operator.token.list.write", "書き込み"),
    msg("operator.token.list.full", "完全なアクセス"),
    msg(
        "operator.account.help.summary",
        "account …      アカウントと、アカウントにできることを管理します（create, set-password, disable, enable, link-oidc, unlink-oidc, grant, revoke, list, show）。これらは、動いているサーバーのそばで実行できます。詳しくは planeter operator account --help を実行してください。",
    ),
    msg(
        "operator.account.help.usage",
        "使い方: planeter operator account create HANDLE --name TEXT [--password-prompt | --password-fd N]\n        planeter operator account set-password HANDLE [--password-fd N]\n        planeter operator account disable HANDLE --reason TEXT\n        planeter operator account enable HANDLE\n        planeter operator account link-oidc HANDLE --issuer URL --subject TEXT\n        planeter operator account unlink-oidc HANDLE --issuer URL --subject TEXT\n        planeter operator account grant OWNER/NAME HANDLE --role read|write|maintain|admin\n        planeter operator account revoke OWNER/NAME HANDLE [--allow-no-admin]\n        planeter operator account list [--check]\n        planeter operator account show HANDLE\n       （どれも [--lang en|ja] を付けられます）\n\ncreate         アカウントを作ります。ユーザー名（HANDLE） は、小文字の英字、数字、- _ . の 1〜39 文字で、英字か数字から始め、あとから変えられません\n--name         アカウントに表示する名前。1〜100 文字\nset-password   アカウントのパスワードを設定します。そのアカウントのログイン中の状態はすべて終わり、トークンには手を付けません\ndisable        アカウントをすぐに止めます。ログインできなくなり、ログイン中の状態は終わり、トークンは受け付けられません。何も削除しません。--reason に理由を 1〜200 文字で書きます\nenable         disable を元に戻します。トークンは有効期限まで再び使え、終わったログイン中の状態は戻りません\nlink-oidc      アカウントを、このサーバーが設定されているログインのプロバイダー（PLANETER_OIDC_ISSUER） の ID に結び付けます。ほかのアカウントの ID を移すことはしません\ngrant          アカウントに、リポジトリでの権限を 1 つだけ与えます\nrevoke         アカウント自身がリポジトリで持っている権限を取り上げます。リポジトリの管理者が最後の 1 人になるときは、--allow-no-admin を付けないかぎり、取り上げません\nlist           アカウントを、ユーザー名の順に表示します。--check を付けると、大文字と小文字だけが違うユーザー名を表示します\nshow           アカウント 1 つ: 状態、権限、直近の 50 件の出来事",
    ),
    msg(
        "operator.account.help.roles",
        "権限: read: リポジトリを見て、クローンできます。write: さらに、作業を提案できます。maintain: さらに、引き渡しを依頼し、提案をレビューできます。admin: さらに、だれがアクセスできるかを管理できます",
    ),
    msg(
        "operator.account.help.password",
        "パスワード: パスワードは、端末で画面に表示されない形で 2 回入力するか（--password-prompt。set-password では --password-fd がなければ常にこちら）、スクリプト向けに、開いているファイル記述子から 1 行として読みます（--password-fd N。スクリプトが、先にその番号でファイルかパイプを開いておきます）。引数や環境変数では渡せません。端末に入力が表示されなくなったときは、stty sane を実行してください。",
    ),
    msg(
        "operator.account.help.exit_codes",
        "終了コード: 0 完了。1 拒否（何も変えていません）。2 コマンドライン、または、パスワードを入力する端末がない。5 ローカルのファイルまたはデータベース（5 秒を超えて使用中のデータベースは、その旨を表示します）。これらのコマンドはサーバーを止めなくても実行できるので、3 は使いません。1 回の操作は 1 つの手順なので、変更が途中で終わることはなく、4 も使いません",
    ),
    msg(
        "operator.account.error.usage",
        "このコマンドラインは解釈できません。planeter operator account --help を実行して、指定できるものを確認してください。",
    ),
    msg(
        "operator.account.error.handle_shape",
        "ユーザー名 {handle} は使えません。ユーザー名は、小文字の英字、数字、- _ . の 1〜39 文字で、英字か数字から始め、.. を含まず、末尾が . ではないものにしてください。",
    ),
    msg(
        "operator.account.error.handle_upper",
        "ユーザー名は小文字で書きます。{handle} には大文字が含まれています。小文字にして、もう一度実行してください。",
    ),
    msg(
        "operator.account.error.handle_reserved",
        "{handle} は、このサーバーが自分のページに使っている言葉です。別のユーザー名にしてください。",
    ),
    msg(
        "operator.account.error.handle_taken",
        "ユーザー名 {handle} はすでに使われています（大文字と小文字は区別せずに比べ、組織とリポジトリの所有者も数えます）。別のユーザー名にしてください。",
    ),
    msg(
        "operator.account.error.name_missing",
        "表示名が必要です。--name TEXT に、1〜{max} 文字で書いてください。",
    ),
    msg(
        "operator.account.error.name_long",
        "表示名は {length} 文字あり、{max} 文字までです。短くして、もう一度実行してください。",
    ),
    msg(
        "operator.account.error.name_unsafe",
        "表示名に、改行など、表示できない文字が含まれています。普通の文章にして、もう一度実行してください。",
    ),
    msg(
        "operator.account.error.password_short",
        "パスワードが短すぎます。{min} 文字以上にしてください。何も変えていません。",
    ),
    msg(
        "operator.account.error.password_long",
        "パスワードが長すぎます。{max} 文字までにしてください。何も変えていません。",
    ),
    msg(
        "operator.account.error.password_handle",
        "パスワードがユーザー名と同じです。別のものにしてください。何も変えていません。",
    ),
    msg(
        "operator.account.error.password_common",
        "そのパスワードは、とてもよく使われるパスワードの 1 つです。別のものにしてください。何も変えていません。",
    ),
    msg(
        "operator.account.error.password_differ",
        "2 回の入力が同じではありません。何も変えていません。もう一度実行してください。",
    ),
    msg(
        "operator.account.error.password_no_terminal",
        "標準入力が端末ではないので、パスワードを入力できません。端末で実行するか、--password-fd N でファイル記述子からパスワードを渡してください。",
    ),
    msg(
        "operator.account.error.password_fd",
        "ファイル記述子 {fd} から、パスワードの 1 行を読めませんでした: {why}。何も変えていません。",
    ),
    msg(
        "operator.account.error.password_echo",
        "端末の入力表示を止められなかった ({why}) ので、パスワードは尋ねませんでした。--password-fd N を使うか、stty が使える場所で実行してください。何も変えていません。",
    ),
    msg("operator.account.prompt.password", "パスワード:"),
    msg("operator.account.prompt.again", "もう一度パスワード:"),
    msg(
        "operator.account.error.no_account",
        "アカウント {handle} はありません。ユーザー名を確認してください。",
    ),
    msg(
        "operator.account.error.reason_missing",
        "理由が必要です。アカウントを無効にする理由を、1〜{max} 文字で書いてください。",
    ),
    msg(
        "operator.account.error.already_disabled",
        "{handle} はすでに無効になっています。何も変えていません。",
    ),
    msg(
        "operator.account.error.not_disabled",
        "{handle} は無効になっていません。何も変えていません。",
    ),
    msg(
        "operator.account.error.no_provider",
        "このサーバーには、ログインのプロバイダーが設定されていません（PLANETER_OIDC_ISSUER が未設定です） ので、結び付ける ID がありません。サーバーと同じように設定して、もう一度実行してください。",
    ),
    msg(
        "operator.account.error.other_issuer",
        "{issuer} は、このサーバーが設定されているログインのプロバイダーではありません。PLANETER_OIDC_ISSUER の値を使ってください。",
    ),
    msg(
        "operator.account.error.identity_taken",
        "この ID はすでにほかのアカウントに結び付いていて、移すことはしません。先にそちらで結び付けを外してください。",
    ),
    msg(
        "operator.account.error.not_linked",
        "この ID は {handle} に結び付いていません。何も変えていません。",
    ),
    msg(
        "operator.account.error.already_linked",
        "この ID はすでに {handle} に結び付いています。何も変えていません。",
    ),
    msg(
        "operator.account.error.role",
        "権限は read、write、maintain、admin のどれかにしてください。",
    ),
    msg(
        "operator.account.error.account_disabled",
        "{handle} は無効になっているので、権限を与えられません。先に有効にしてください。",
    ),
    msg(
        "operator.account.error.last_admin",
        "そうすると、{repo} を管理できる人がいなくなります。先にほかのアカウントをリポジトリの管理者にするか、いなくてもよいなら --allow-no-admin を付けてください。",
    ),
    msg(
        "operator.account.error.no_grant",
        "{handle} は {repo} に自分の権限を持っていないので、取り上げるものがありません。",
    ),
    msg(
        "operator.account.error.already_role",
        "{handle} は、すでに {repo} で {role} の権限を持っています。何も変えていません。",
    ),
    msg(
        "operator.account.error.busy",
        "データベースが {seconds} 秒を超えて使用中でした（ほかのプロセスが書き込んでいます）。何も変えていません。もう一度実行してください。",
    ),
    msg(
        "operator.account.done.has_password",
        "{handle} ({name}) には、パスワードがあります。",
    ),
    msg(
        "operator.account.done.no_password",
        "{handle} ({name}) には、パスワードがありません。結び付けた ID でだけログインできます。",
    ),
    msg(
        "operator.account.done.sessions_ended",
        "ログイン中の状態はすべて終わりました。トークンには手を付けていません。",
    ),
    msg(
        "operator.account.done.disabled",
        "ログイン中の状態はすぐに終わり、トークンは受け付けられません。持っているものは何も削除されておらず、ほかの人はリポジトリを引き続き読めます。",
    ),
    msg(
        "operator.account.done.enabled",
        "トークンは、有効期限まで再び使えます。終わったログイン中の状態は戻りません。",
    ),
    msg(
        "operator.account.done.linked",
        "{handle} は、そのプロバイダーでログインできるようになりました。",
    ),
    msg(
        "operator.account.done.unlinked",
        "{handle} は、その ID でログインできなくなりました。",
    ),
    msg(
        "operator.account.done.granted",
        "{name} ({handle}) に、{repo} の {role} が与えられました: {means}",
    ),
    msg(
        "operator.account.done.revoked",
        "{name} ({handle}) は、{repo} で自分の {role} の権限を持たなくなりました。",
    ),
    msg(
        "operator.account.done.no_admin",
        "{repo} は、リポジトリの管理者を与えるまで、だれも管理できません。",
    ),
    msg(
        "operator.account.role.read",
        "リポジトリを見て、クローンできます",
    ),
    msg("operator.account.role.write", "さらに、作業を提案できます"),
    msg(
        "operator.account.role.maintain",
        "さらに、引き渡しを依頼し、提案をレビューできます",
    ),
    msg(
        "operator.account.role.admin",
        "さらに、だれがアクセスできるかを管理できます",
    ),
    msg(
        "operator.account.warn.sole_admin",
        "次のリポジトリは、リポジトリの管理者を与えるまで、だれも管理できなくなります: {repos}",
    ),
    msg(
        "operator.account.warn.sole_admin_more",
        "次のリポジトリは、リポジトリの管理者を与えるまで、だれも管理できなくなります: {repos}、ほか {more} 件",
    ),
    msg(
        "operator.account.list.header",
        "ユーザー名 | 表示名 | 状態 | パスワード | ID | 作成日",
    ),
    msg("operator.account.list.active", "有効"),
    msg("operator.account.list.disabled", "無効"),
    msg("operator.account.list.yes", "あり"),
    msg("operator.account.list.no", "なし"),
    msg("operator.account.list.none", "アカウントはありません。"),
    msg(
        "operator.account.list.more",
        "ほか {more} 件（全部で {total} 件）",
    ),
    msg(
        "operator.account.check.none",
        "大文字と小文字だけが違うユーザー名はありません。",
    ),
    msg(
        "operator.account.check.explain",
        "次のユーザー名は、大文字と小文字だけが違います。これまでどおり使え、名前は変えていません:",
    ),
    msg("operator.account.check.group", "{key}: {handles}"),
    msg(
        "operator.account.check.more",
        "ほか {more} 行（全部で {total} 行）",
    ),
    msg("operator.account.show.handle", "ユーザー名"),
    msg("operator.account.show.name", "表示名"),
    msg("operator.account.show.state", "状態"),
    msg(
        "operator.account.show.disabled",
        "{date} に無効にしました: {reason}",
    ),
    msg("operator.account.show.created", "作成日"),
    msg("operator.account.show.password", "パスワード"),
    msg(
        "operator.account.show.password_changed",
        "あり（{date} に設定）",
    ),
    msg("operator.account.show.identities", "結び付いた ID"),
    msg("operator.account.show.roles", "権限"),
    msg("operator.account.show.roles_none", "自分のものはありません"),
    msg(
        "operator.account.show.roles_more",
        "ほか {more} 件（全部で {total} 件）",
    ),
    msg("operator.account.show.owner", "所有者"),
    msg("operator.account.show.events", "直近の出来事"),
    msg("operator.account.show.events_none", "まだありません"),
    msg("operator.account.show.event_line", "{date}  {sentence}"),
    msg("account.by.self", "あなた"),
    msg("account.by.operator", "このサーバーを運用している人"),
    msg("account.by.provider", "ログインのプロバイダー"),
    msg(
        "account.event.created",
        "アカウントは、{by}が作成しました。",
    ),
    msg(
        "account.event.password_set",
        "パスワードを、{by}が設定しました。",
    ),
    msg(
        "account.event.password_changed",
        "パスワードを、{by}が変更しました。",
    ),
    msg(
        "account.event.sign_ins_ended",
        "ログイン中の状態を {count} 件、終わらせました。",
    ),
    msg("account.event.then", "{first}{second}"),
    msg(
        "account.event.identity_linked",
        "{host} の ID を、{by}がアカウントに結び付けました。",
    ),
    msg(
        "account.event.identity_unlinked",
        "{host} の ID を、{by}がアカウントから外しました。",
    ),
    msg(
        "account.event.plain.identity_linked",
        "ID を、{by}がアカウントに結び付けました。",
    ),
    msg(
        "account.event.plain.identity_unlinked",
        "ID を、{by}がアカウントから外しました。",
    ),
    msg(
        "account.event.token_created",
        "トークン「{name}」（{scope}、{repositories}、{date} まで）を、{by}が作成しました。",
    ),
    msg(
        "account.event.token_revoked",
        "トークン「{name}」を、{by}が取り消しました。",
    ),
    msg(
        "account.event.plain.token_created",
        "トークンを、{by}が作成しました。",
    ),
    msg(
        "account.event.plain.token_revoked",
        "トークンを、{by}が取り消しました。",
    ),
    msg("account.event.token_repositories", "{count} 件のリポジトリ"),
    msg(
        "account.event.token_repositories_all",
        "アカウントが届くすべてのリポジトリ",
    ),
    msg(
        "account.event.session_ended",
        "ログイン中の状態 {count} 件を、{by}が終わらせました。",
    ),
    msg(
        "account.event.role_granted",
        "{repository} での権限「{role}」を、{by}が与えました。",
    ),
    msg(
        "account.event.role_revoked",
        "{repository} での権限「{role}」を、{by}が取り上げました。",
    ),
    msg(
        "account.event.plain.role_granted",
        "リポジトリでの権限を、{by}が与えました。",
    ),
    msg(
        "account.event.plain.role_revoked",
        "リポジトリでの権限を、{by}が取り上げました。",
    ),
    msg("account.event.role.read", "読み取り"),
    msg("account.event.role.write", "書き込み"),
    msg("account.event.role.maintain", "メンテナー"),
    msg("account.event.role.admin", "リポジトリの管理者"),
    msg(
        "account.event.disabled",
        "アカウントを、{by}が無効にしました: {reason}",
    ),
    msg(
        "account.event.plain.disabled",
        "アカウントを、{by}が無効にしました。",
    ),
    msg(
        "account.event.enabled",
        "アカウントを、{by}が有効にしました。",
    ),
    msg(
        "account.event.signed_in_new_method",
        "アカウントが、これまで使っていなかった方法でログインしました。",
    ),
    msg(
        "provider.error.allow_private_value",
        "PLANETER_OIDC_ALLOW_PRIVATE_ISSUER は on か off にしてください。今の値は {value} です。on にするか、設定を外してください。",
    ),
    msg(
        "provider.error.allow_private_no_provider",
        "PLANETER_OIDC_ALLOW_PRIVATE_ISSUER=on には、ログインのプロバイダーが必要です。PLANETER_OIDC_ISSUER を設定するか、この設定を外してください。",
    ),
    msg(
        "provider.error.issuer_host",
        "PLANETER_OIDC_ISSUER {issuer} には、許可できるホストがありません。アドレスを確認してください。",
    ),
    msg(
        "provider.error.session_hours",
        "PLANETER_SESSION_HOURS は、{min} から {max} までの整数の時間にしてください。今の値は {value} です。その範囲の数にするか、設定を外してください。",
    ),
    msg(
        "provider.line.allow_private",
        "ログインのプロバイダーのホスト {host} は、プライベートアドレスにあることが許可されています（PLANETER_OIDC_ALLOW_PRIVATE_ISSUER=on）。それ以外の接続先は、これまでどおり確認されます。",
    ),
    msg(
        "provider.line.session_hours",
        "ログイン中の状態は {count} 時間続きます。",
    ),
];
