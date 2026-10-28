//! The Japanese catalog (RFC 012 D-3, handoff `japanese-and-switcher-handoff-v1`): **a draft for the reviewers**.
//!
//! Every message is `needs-review` (see `.git-exclude/review-requests/japanese-review-sheet.md`); the language is
//! not in [`super::shipped`] until [`super::JA_REVIEWED`] is `true`, which only the reviewers' pass allows. Until
//! then it is negotiable only when the operator turns the preview on (`PLANETER_PREVIEW_LANGUAGES=ja`).
//!
//! Style: polite form (です・ます); headings, labels and badges are noun phrases; Japanese has no grammatical
//! plural, so a message with a count says the same in both forms; the order follows [`super::Key`] and the
//! English catalog, key for key, with the same placeholders. The terms are the Japanese column of
//! `docs/glossary.md` (section 5); the **hard terms** are flagged there for the reviewers' decision.

use super::{Entry, msg, plural_msg};

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
        "今はこのリポジトリを読み込めません。しばらくしてから、もう一度お試しください。それでも解決しない場合は、このサーバーの管理者にお知らせください。",
    ),
    msg("busy.heading", "サーバーが混み合っています"),
    msg(
        "busy.body",
        "現在、多くのリクエストを処理しています。しばらくしてから、もう一度お試しください。",
    ),
    msg("too_large.heading", "大きすぎて表示できません"),
    msg(
        "too_large.body",
        "これは大きすぎて、このサーバーでは表示できません。ダウンロードするか、このサーバーの管理者に上限の引き上げを依頼してください。",
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
    msg("assurance.queued", "承認済み・署名待ち"),
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
        "この変更は、このサーバーで承認されました。メンテナーがまだプロジェクトの履歴に署名していないため、検証済みの記録には含まれていません。",
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
    msg("explain.patch", "パッチとは、記録された編集1件のことです。"),
    msg(
        "explain.rollback",
        "ロールバックとは、以前の変更を取り消す変更のことです。",
    ),
    // history, change, refs, verify
    msg("history.heading", "履歴 {ref}"),
    msg("history.rollback", "ロールバック"),
    plural_msg(
        "history.patch_count",
        "パッチ {count} 件",
        "パッチ {count} 件",
    ),
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
    msg("refs.closed", "クローズ済み"),
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
        "このサーバーは、そこまで大きなファイルを提供しません。このサーバーの管理者に問い合わせてください。",
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
        "このサーバーの管理者に、エクスポート用スプールの設定を依頼してください。",
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
        "エクスポートがこのサーバーの許す時間を超えました。このサーバーの管理者に、エクスポートの制限時間の引き上げを依頼してください。",
    ),
    msg(
        "error.export_failed.ceiling_hint",
        "エクスポートに必要なメモリが、このサーバーの許す量を超えています。このサーバーの管理者に、エクスポート用メモリ上限の引き上げを依頼してください。",
    ),
    msg(
        "error.export_failed.spool_hint",
        "アーティファクトがエクスポート用スプール全体より大きくなっています。このサーバーの管理者に、スプールの拡大を依頼してください。",
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
        "このリポジトリで開いている提案がすでに{used}件あり、上限は{limit}件です。",
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
        "この提案にはすでに{used}件のリビジョンがあり、上限は{limit}件です。",
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
        "メンテナーに完了した提案を閉じるよう依頼するか、このサーバーの管理者に容量の引き上げを依頼してください。",
    ),
    // making a proposal (RFC 004 part 3b)
    msg(
        "error.forbidden.message",
        "このリポジトリは読めますが、提案する権限はありません。",
    ),
    msg(
        "error.forbidden.hint",
        "メンテナーに書き込み権限を依頼するか、サインインしている全員に提案を開放するよう依頼してください。",
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
        "もっと小さいアーティファクトを送るか、このサーバーの管理者に上限の引き上げを依頼してください。",
    ),
    msg(
        "error.needs_recovery.message",
        "このリポジトリは、完了しなかった書き込みの復旧を待っているため、今は新しい提案を受け付けません。",
    ),
    msg(
        "error.needs_recovery.hint",
        "しばらくしてからもう一度お試しください。読み取りには影響がなく、理由はこのサーバーの管理者がサーバーのログで確認できます。",
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
        "このサーバーの管理者に、アーティファクト保存先の設定を依頼してください。",
    ),
    msg(
        "propose.held",
        "あなたの作業はこのサーバーでレビュー待ちとして保持されており、どのブランチも変更されていません。",
    ),
    msg(
        "propose.already_recorded",
        "このアーティファクトはすでに記録済みのため何も追加されておらず、どのブランチも変更されていません。",
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
        "リポジトリのツールの出力を読み取れなかったため、何も変更していません。",
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
    plural_msg(
        "recovery.note.unreached_objects",
        "リポジトリの点検で、どこからも使われていない保存データが {count} 件見つかりました。復旧は止まらず、対応は不要です。",
        "リポジトリの点検で、どこからも使われていない保存データが {count} 件見つかりました。復旧は止まらず、対応は不要です。",
    ),
    msg(
        "recovery.outcome.recovered",
        "リポジトリは再び書き込めるようになりました。",
    ),
    msg(
        "recovery.outcome.stopped",
        "復旧は停止し、これ以上は何も変更していません。フラグはまだ立っています。",
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
        "このシングルサインオンのIDは、planeter のアカウントに連携されていません。このサーバーの管理者に問い合わせてください。",
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
    plural_msg(
        "proposal.summary",
        "{revisions}にパッチ {count} 件",
        "{revisions}にパッチ {count} 件",
    ),
    plural_msg(
        "proposal.revisions_count",
        "リビジョン {count} 件",
        "リビジョン {count} 件",
    ),
    msg("proposal.by", "提案者：{who}"),
    msg("proposal.when", "最終更新：{when}"),
    msg("proposal.closed_reason", "理由：{reason}"),
    msg("proposal.details", "詳細"),
    msg("proposal.description", "説明"),
    msg("proposal.no_description", "説明は書かれていません。"),
    msg(
        "proposal.file_view_missing",
        "ファイルごとの表示はまだ利用できません。",
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
    plural_msg(
        "proposal.patches.pending",
        "{count} 件のパッチは、まだこのサーバーが保持しており、署名されていません。",
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
        "このサーバーは、このパッチが署名されて履歴に入ったかどうかを判断できません(履歴が、サーバーが読む範囲より長いためです)。",
    ),
    msg("proposal.patch.not_listed", "この場所には記録なし"),
    msg(
        "proposal.patch.not_listed_explain",
        "このパッチは、このサーバーが保持しているものにも、保持している履歴にもないため、何も言えません。",
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
    plural_msg(
        "repo.open_proposals",
        "開いている提案 {count} 件",
        "開いている提案 {count} 件",
    ),
    msg("proposal.back", "提案に戻る"),
    msg(
        "proposal.close.too_long",
        "理由が 1,000 文字を超えています。短くして、もう一度お試しください。",
    ),
    msg("proposal.closed_line", "閉じています：{reason}"),
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
        "このリポジトリは読めますが、コメントするにはサインインが必要です。",
    ),
    msg(
        "error.forbidden_comment.hint",
        "サインインするか、アクセストークンを付けてリクエストを送ってください。",
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
        "このリポジトリは読めますが、コメントを非表示にしたり再表示したりできるのは管理者だけです。",
    ),
    msg(
        "error.forbidden_hide.hint",
        "リポジトリの管理者に依頼してください。",
    ),
    msg(
        "error.rate_comments.message",
        "この1時間に、この提案へ何度もコメントしています。",
    ),
    msg(
        "error.rate_comments.hint",
        "しばらく待ってからもう一度お試しください。待ち時間は Retry-After ヘッダーにあります。",
    ),
    msg(
        "error.rate_decisions.message",
        "この1時間に、この提案へ何度も判断を記録しています。",
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
    plural_msg(
        "review.line.approved",
        "{count} 人が承認しました",
        "{count} 人が承認しました",
    ),
    plural_msg(
        "review.line.changes",
        "{count} 人が変更を依頼しています",
        "{count} 人が変更を依頼しています",
    ),
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
    plural_msg(
        "comments.count",
        "コメント {count} 件",
        "コメント {count} 件",
    ),
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
        "これはリビジョン {revision} だけに記録され、このリポジトリを読める全員に見えます。提案が開いている間は変更できます。あとのリビジョンには引き継がれず、何もマージしません。",
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
    msg("hide.refused.heading", "コメントは変更されませんでした"),
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
        "確認待ちのアーカイブがすでに {limit} 件あります。",
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
        "このアーカイブから作られるリポジトリは prikk の検査に通らないため、作成されませんでした。",
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
        "この応答の field と reason が断られた値と理由を示しているので、直して、API 文書の項目の通りにもう一度送ってください。",
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
        "error.import_length.message",
        "リクエストの長さが示されていません。",
    ),
    msg(
        "error.import_length.hint",
        "Content-Length ヘッダーを付けてアーカイブを送ってください。",
    ),
    msg(
        "error.invalid_request.import_framing_hint",
        "本文は PLANETER-IMPORT-1 で始め、続けてメタデータの長さ、メタデータ、アーカイブの順に、API 文書の通りに送ってください。",
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
        "誰でも（サインインの有無を問わない）",
    ),
    msg("new.visibility.internal", "サインインしている人なら誰でも"),
    msg("new.visibility.private", "アクセスを与えた人だけ"),
    msg("new.proposals", "提案を送れる人"),
    msg("new.proposals.members", "書き込み権限を与えた人"),
    msg("new.proposals.any", "読めるサインイン済みの人なら誰でも"),
    msg(
        "new.proposals.any_explain",
        "知らない人からの貢献を受け付ける公開プロジェクトでは、後者を選びます。その人たちの作業はレビュー待ちで保持され、それだけでブランチが変わることはありません。",
    ),
    msg("new.empty.heading", "空のリポジトリ"),
    msg("new.empty.explain", "履歴なしで始めます。"),
    msg("new.empty.button", "空のリポジトリを作成する"),
    msg(
        "new.empty.consequence",
        "リポジトリをいま作成します。あなたがその管理者になり、上で選んだ人が読めるようになります。",
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
    plural_msg(
        "stage.objects",
        "全部で {count} 件の項目を含みます。",
        "全部で {count} 件の項目を含みます。",
    ),
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
    plural_msg(
        "stage.ref.more",
        "ほか {count} 件のブランチ",
        "ほか {count} 件のブランチ",
    ),
    msg(
        "stage.verifies.yes",
        "下の鍵をすべて信頼すれば、新しいリポジトリは prikk のすべての検査に通ります。",
    ),
    msg(
        "stage.verifies.no",
        "下の鍵をすべて信頼しても、新しいリポジトリは prikk の検査に通りません（{problems}）。鍵を 1 つも信頼せずに作成することはできます。",
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
        "リポジトリは prikk のすべての検査に通りました。",
    ),
    msg(
        "created.unverified",
        "信頼しなかった鍵で署名されたブランチがあるため、リポジトリは prikk のすべての検査には通りません。鍵を信頼するまで何も検証・信頼されず、このバージョンでは後から鍵を信頼することもできません。",
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
        "リポジトリを作成するには、サインインしてください。",
    ),
    msg("new.nav", "新しいリポジトリ"),
    msg("stage.prikk", "prikk の説明：{detail}"),
    msg(
        "stage.error.both",
        "鍵 {key} について、フィンガープリントを入力し、同時に信頼しないことも選んでいます。どちらか一方にしてください。",
    ),
    msg("stage.error.verify", "失敗した検査：{conditions}。"),
    msg(
        "stage.gone",
        "そのアーカイブはもうありません。すでに使われたか、保管期間が過ぎた可能性があります。",
    ),
    msg("stage.gone.link", "リポジトリを作成する"),
];
