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
];
