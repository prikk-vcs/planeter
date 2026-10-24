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
];
