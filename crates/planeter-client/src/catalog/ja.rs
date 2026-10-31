//! Japanese: a **draft** for the reviewers (the owner and a colleague; RFC 012 D-3f). Preview only until `JA_REVIEWED` is `true`:
//! used with `--lang ja`, never chosen from `LANG`. The words follow the server's Japanese glossary (`docs/glossary.md` §5):
//! 提案, リビジョン, 保持, 鍵を信頼する; `このサーバーを運用している人`, never `管理者` alone.

use super::{Entry, msg};

pub static CATALOG: &[Entry] = &[
    msg(
        "help.main",
        "planeter-client は、あなた自身の prikk と鍵を使って planeter サーバーとやり取りします。鍵を持つことはなく、サーバー上のブランチを変えることもありません。\n\n使い方:\n  planeter-client propose <https://host/owner/name> [--ref heads/NAME] [--proposal ID] [--title TITLE] [--description-file FILE] [--dry-run] [--retry]\n  planeter-client clone <https://host/owner/name> [DIR] [--trust-key KEY]... [--no-adopt]\n  planeter-client fetch [DIR]\n\nすべてのコマンドに共通:\n  --token-file FILE   トークンを FILE から読みます（または PLANETER_TOKEN を設定）。コマンドラインからトークンを受け取ることはありません\n  --cacert FILE       FILE にある認証局だけを信頼します。内蔵の一覧は置き換えられます\n  --lang en|ja        メッセージの言語（日本語は草案です）\n  --verbose           メッセージの背後にある元の理由も表示します\n  --help, --version\n\n終了コード: 0 完了、1 サーバーが断った、2 コマンドラインの誤り、3 ネットワークまたは TLS、4 prikk の失敗、5 ローカルのファイルまたは鍵。\n\n例:\n  planeter-client propose https://forge.example/alice/app --title \"パーサーを直す\" --dry-run",
    ),
    msg(
        "help.propose",
        "planeter-client propose <https://host/owner/name> [オプション]\n\nサーバーがすでに持っているものを土台に、あなたの prikk であなたの作業を組み立て、提案として送ります。サーバー側で署名やマージは行われません。作業はレビュー待ちとして保持されます。\n\n  --ref heads/NAME          送る自分のブランチ（既定は heads/main）\n  --proposal ID             新しい提案を開く代わりに、この提案へリビジョンを追加します\n  --title TITLE             新しい提案のタイトル（新規の場合は必須）\n  --description-file FILE   説明を FILE から読みます\n  --dry-run                 組み立てて、送る内容を表示するだけで、何も送りません\n  --retry                   サーバーが取り込みを終えられなかったとき、同じ作業をもう一度送ります\n  --token-file FILE, --cacert FILE, --lang, --verbose\n\n自分の prikk リポジトリの中で実行してください。\n\n例:\n  planeter-client propose https://forge.example/alice/app --title \"パーサーを直す\"",
    ),
    msg(
        "help.clone",
        "planeter-client clone <https://host/owner/name> [DIR] [オプション]\n\nリポジトリをダウンロードし、あなたの prikk で確認して、含まれているメンテナーの鍵を表示します。信頼されるのは、あなたが入力した鍵だけです。\n\n  DIR                 置き場所（既定はリポジトリの名前）\n  --trust-key KEY     この公開鍵全体を信頼します。入力できる端末がないときは、鍵ごとに 1 回ずつ指定します\n  --no-adopt          すべてを受信したコピーとして保持し、何も信頼しません\n  --token-file FILE, --cacert FILE, --lang, --verbose\n\n例:\n  planeter-client clone https://forge.example/alice/app",
    ),
    msg(
        "help.fetch",
        "planeter-client fetch [DIR] [オプション]\n\nサーバーに変更の有無をたずね、あれば取得し、あなたの prikk で確認して、受信したコピーとして取り込みます。あなた自身のブランチは変わりません。\n\n  DIR   リポジトリ（既定は現在のディレクトリ）\n  --token-file FILE, --cacert FILE, --lang, --verbose\n\n例:\n  planeter-client fetch",
    ),
    msg(
        "version.line",
        "planeter-client {version}（ビルド {commit}）。prikk {floor} 以降が必要です",
    ),
    msg(
        "usage.unknown_command",
        "「{name}」というコマンドはありません。planeter-client --help でコマンドを確認してください。",
    ),
    msg(
        "usage.unknown_option",
        "「{name}」はこのコマンドのオプションではありません。planeter-client {command} --help で確認してください。",
    ),
    msg(
        "usage.missing_value",
        "オプション {name} には値が必要です。例は planeter-client {command} --help にあります。",
    ),
    msg(
        "usage.missing_argument",
        "このコマンドには{what}が必要です。例は planeter-client {command} --help にあります。",
    ),
    msg(
        "usage.too_many",
        "このコマンドが受け取る以上のものがコマンドラインにあります（「{extra}」）。planeter-client {command} --help を確認してください。",
    ),
    msg(
        "usage.bad_lang",
        "「{value}」はこのプログラムにない言語です。en か ja を指定してください。",
    ),
    msg(
        "usage.bad_url",
        "「{url}」はリポジトリのアドレスではありません。https://host/owner/name の形で指定してください。",
    ),
    msg(
        "usage.plain_http",
        "「{url}」は http:// で始まっていますが、このプログラムは https でしかサーバーと通信しません。https:// のアドレスを指定してください。",
    ),
    msg(
        "usage.no_repository",
        "「{dir}」は prikk のリポジトリではありません。自分のリポジトリの中で実行するか、リポジトリを指定してください。",
    ),
    msg(
        "token.required",
        "提案にはトークンが必要ですが、指定されていません。PLANETER_TOKEN を設定するか、--token-file FILE を指定してください。",
    ),
    msg(
        "token.file_unreadable",
        "トークンのファイル {path} を読み込めませんでした。パスと読み取りの権限を確認してください。",
    ),
    msg(
        "token.file_too_open",
        "トークンのファイル {path} はほかのユーザーにも読めるため、使用しません。chmod 600 を実行してから、もう一度お試しください。",
    ),
    msg(
        "token.file_empty",
        "トークンのファイル {path} が空です。トークンを 1 行で書き込んでください。",
    ),
    msg(
        "tls.unknown_issuer",
        "このサーバーの証明書は、このプログラムが信頼する認証局に署名されていません。サーバーが独自の認証局を使っているとわかっている場合は、その認証局の証明書を --cacert FILE で指定してください。",
    ),
    msg(
        "tls.wrong_name",
        "このサーバーの証明書は、アドレスにある名前とは別の名前のものです。アドレスを確認するか、このサーバーを運用している人にたずねてください。",
    ),
    msg(
        "tls.expired",
        "このサーバーの証明書は期限が切れています（またはまだ有効ではありません）。このサーバーを運用している人に伝えてください。お使いのコンピューターの時計も確認してください。",
    ),
    msg(
        "tls.other",
        "サーバーとの安全な接続を作れませんでした。--verbose を付けて実行すると理由が表示されます。このサーバーを運用している人に伝えてください。",
    ),
    msg(
        "net.unreachable",
        "{host} のサーバーに接続できませんでした。アドレスと接続を確認してから、もう一度お試しください。",
    ),
    msg(
        "net.timeout",
        "{host} のサーバーが時間内に応答しませんでした。しばらくしてから、もう一度お試しください。",
    ),
    msg(
        "net.cut_short",
        "応答がすべて届く前に接続が切れました。同じコマンドをもう一度実行すると、続きから取得します。",
    ),
    msg(
        "net.redirect",
        "サーバーが {location} へのリダイレクトを返しましたが、このプログラムはリダイレクトには従いません。指定したアドレスを確認してください。",
    ),
    msg(
        "net.too_long",
        "サーバーが予告より多くのデータを送ってきたため、応答を破棄しました。もう一度お試しください。続く場合は、このサーバーを運用している人に伝えてください。",
    ),
    msg(
        "net.other",
        "サーバーへのリクエストに失敗しました。--verbose を付けて実行すると理由が表示されます。",
    ),
    msg(
        "net.bad_answer",
        "サーバーの応答を理解できませんでした。アドレスが planeter サーバーのものか確認して、もう一度お試しください。",
    ),
    msg(
        "net.cacert_unreadable",
        "証明書のファイル {path} を証明書として読み込めませんでした。認証局の証明書を含む PEM ファイルを指定してください。",
    ),
    msg(
        "net.proxy",
        "HTTPS_PROXY に指定されたプロキシを使えませんでした。変数を確認するか、このサーバーについて NO_PROXY を設定してください。",
    ),
    msg("server.refused", "サーバーが拒否しました：{message} {hint}"),
    msg(
        "server.refused_plain",
        "サーバーがリクエストを拒否しました（{status}）が、理由は示されませんでした。もう一度お試しください。続く場合は、このサーバーを運用している人に伝えてください。",
    ),
    msg(
        "server.not_found",
        "そのリポジトリはサーバーにないか、あなたには読めません。アドレスと、使っているトークンが正しいかを確認してください。",
    ),
    msg(
        "server.forbidden",
        "このリポジトリでは、この操作はできません。メンテナーに、提案する権限を付けてもらってください。",
    ),
    msg(
        "server.sign_in",
        "サーバーがトークンを受け付けませんでした。トークンが有効で、このサーバー用に作られたものか確認してください。",
    ),
    msg(
        "server.rate_limited",
        "ここからサーバーへのリクエストが多すぎます。{seconds} 秒待ってから、もう一度お試しください。",
    ),
    msg(
        "server.busy",
        "サーバーが混み合っています。{seconds} 秒待ってから、同じコマンドをもう一度実行してください。",
    ),
    msg(
        "server.off",
        "このサーバーは、リポジトリの配布や受け取りが設定されていません。このサーバーを運用している人に伝えてください。",
    ),
    msg(
        "server.too_large",
        "サーバーは一度にこれほど多くは受け取りません。作業を小さくして送るか、このサーバーを運用している人に上限の引き上げを依頼してください。",
    ),
    msg(
        "server.unavailable",
        "サーバーは今はこの操作ができません。しばらくしてから、もう一度お試しください。",
    ),
    msg(
        "propose.step.advertise",
        "{server} がすでに持っているものをたずねています…",
    ),
    msg(
        "propose.step.have",
        "{ref} についてサーバーが持っているものを取得しています…",
    ),
    msg(
        "propose.step.build",
        "あなたの prikk で、それを土台に作業を組み立てています…",
    ),
    msg(
        "propose.step.declare",
        "{base} の上に、新しいパッチが {count} 件あります。",
    ),
    msg("propose.step.send", "{size} を送っています…"),
    msg(
        "propose.done_new",
        "提案 #{id}（リビジョン {revision}）として提案しました。作業はこのサーバーでレビュー待ちとして保持されます。ブランチは変わっていません。レビュアーは {url} で見られます。",
    ),
    msg(
        "propose.done_revision",
        "提案 #{id} のリビジョン {revision} として追加しました。作業はこのサーバーでレビュー待ちとして保持されます。ブランチは変わっていません。レビュアーは {url} で見られます。",
    ),
    msg(
        "propose.already",
        "この作業は、提案 #{id} のリビジョン {revision} としてすでに記録されているため、新しく追加されたものはありません。レビュアーは {url} で見られます。",
    ),
    msg(
        "propose.nothing_new",
        "{ref} にあるものはすべてサーバーがすでに持っているため、提案するものはありません。",
    ),
    msg(
        "propose.dry_run",
        "試行のみ：何も送っていません。{base} の上のパッチ {count} 件（{size}）を {url} に提案することになります。",
    ),
    msg("propose.dry_run_details", "パッチ：{patches}。"),
    msg(
        "propose.no_have",
        "{ref} についてサーバーが持っているものの一覧が提供されていないため、それを土台に作業を組み立てられません。ブランチ名を確認してください。一覧があるのはサーバーのブランチだけです。",
    ),
    msg(
        "propose.branch_gone",
        "あなたのリポジトリには、ブランチ {ref} がありません。送りたいブランチを --ref で指定してください。",
    ),
    msg(
        "propose.base_not_in_history",
        "サーバーの {ref} は、あなたの {ref} に含まれていない地点にあります。サーバーの状態を取得し、その上にあなた自身で作業を載せてから、もう一度提案してください。",
    ),
    msg(
        "propose.over_bound",
        "作業は {size} ありますが、このサーバーが一度に受け取るのは最大 {limit} です。小さく分けて送るか、このサーバーを運用している人に上限の引き上げを依頼してください。",
    ),
    msg(
        "propose.too_many_objects",
        "新しいパッチが {count} 件ありますが、このサーバーが一度に受け取るのは最大 {limit} 件です。小さく分けて送るか、このサーバーを運用している人に上限の引き上げを依頼してください。",
    ),
    msg(
        "propose.title_needed",
        "新しい提案にはタイトルが必要です。--title \"…\" を付けるか、既存の提案に追加する場合は --proposal ID を付けてください。",
    ),
    msg(
        "propose.incomplete",
        "サーバーは作業の取り込みを終えられませんでした。リポジトリが復旧してから、同じ作業をもう一度送ってください。",
    ),
    msg(
        "propose.incomplete_retry",
        "同じコマンドに --retry を付けて実行すると、もう一度送ります。",
    ),
    msg(
        "propose.quota",
        "このサーバーの上限に達しました。{quota} の上限は {limit} {unit} で、{used} を使っています。",
    ),
    msg(
        "propose.quota_remedy",
        "メンテナーに、完了した提案を閉じるか、上限を引き上げるよう依頼してください。",
    ),
    msg(
        "propose.needs_recovery",
        "サーバーはこのリポジトリを修復中で、終わるまで新しい作業を受け付けません。しばらくしてから、もう一度お試しください。",
    ),
    msg(
        "propose.declaration",
        "このプログラムが申告したパッチの一覧が、アーティファクトの内容と一致しません。コマンドをもう一度実行してください。続く場合は、このサーバーを運用している人に伝えてください。",
    ),
    msg(
        "propose.base_unknown",
        "サーバーは、あなたの作業の土台にした地点を知りません。サーバーの状態を取得し、その上に組み立ててから、もう一度提案してください。",
    ),
    msg(
        "propose.too_large_for_server",
        "サーバーは今、この大きさの作業を受け取れません。小さくして送るか、しばらくしてからもう一度お試しください。",
    ),
    msg(
        "propose.not_open",
        "その提案はもう開いていないため、新しいリビジョンを受け付けません。--title を付けて、新しい提案を開いてください。",
    ),
    msg(
        "propose.not_yours",
        "その提案はほかの人のもので、あなたは追加できません。--title を付けて、自分の提案を開いてください。",
    ),
    msg(
        "propose.unknown_proposal",
        "このリポジトリに、その ID の提案はサーバーにありません。ID を確認してください。",
    ),
    msg(
        "propose.no_patches",
        "{ref} には、サーバーにとって新しいものがありません。提案するものはありません。",
    ),
    msg(
        "propose.accept_refused",
        "サーバーは作業を調べましたが、受け付けませんでした：{reason}",
    ),
    msg(
        "clone.step.download",
        "{server} からリポジトリをダウンロードしています…",
    ),
    msg("clone.step.verify", "あなたの prikk で確認しています…"),
    msg("clone.step.import", "{dir} に取り込んでいます…"),
    msg(
        "clone.keys.header",
        "このリポジトリにはメンテナーの鍵が {count} 件含まれています。信頼されるのは、ここで入力した鍵だけです。スキップした鍵は、だれも信頼しません。",
    ),
    msg(
        "clone.keys.none",
        "このリポジトリにはメンテナーの鍵が含まれていないため、信頼するものはありません。",
    ),
    msg(
        "clone.key.show",
        "{count} 件中 {number} 件目の鍵：\n  {key}",
    ),
    msg(
        "clone.key.ask",
        "{number} 件目の鍵を信頼するには、鍵全体を入力してください（上のグループのまま、スペースの有無は問いません）。スキップするには Enter を押してください：",
    ),
    msg("clone.key.trusted", "{number} 件目の鍵を信頼しました。"),
    msg(
        "clone.key.skipped",
        "{number} 件目の鍵はスキップしました。その鍵が署名したものは信頼されません。",
    ),
    msg(
        "clone.key.wrong",
        "{number} 件目の鍵と一致しません。もう一度入力するか、Enter を押してスキップしてください。",
    ),
    msg(
        "clone.key.not_carried",
        "--trust-key で指定された鍵は、このリポジトリに含まれていません（{key}）。あなたのリポジトリには何も取り込まれていません。メンテナーに鍵を確認してください。",
    ),
    msg(
        "clone.no_terminal",
        "鍵を入力できる端末がなく、--trust-key も指定されていません。何もクローンされていません。信頼する鍵ごとに --trust-key KEY を指定するか、何も信頼しない --no-adopt を指定してください。",
    ),
    msg(
        "clone.no_adopt",
        "すべてを受信したコピーとして保持しました。何も検証・信頼されていません。",
    ),
    msg(
        "clone.done",
        "{name} を {dir} にクローンしました。{count} 件中 {trusted} 件の鍵を信頼しています。",
    ),
    msg(
        "clone.done_next",
        "次の手順：{dir} に移動して prikk で内容を確認し、あとで planeter-client fetch を実行して変更を確認してください。",
    ),
    msg(
        "clone.dir_exists",
        "{dir} はすでにあり、空ではありません。別のディレクトリを指定するか、先に削除してください。",
    ),
    msg(
        "clone.verify_failed",
        "ダウンロードしたリポジトリをあなたの prikk で確認しましたが、通りませんでした。{dir} には何も置いていません。このサーバーを運用している人に伝えてください。",
    ),
    msg(
        "clone.archive_bad",
        "ダウンロードしたファイルは、あなたの prikk が読めるリポジトリのアーカイブではありません。{dir} には何も置いていません。",
    ),
    msg("fetch.step.ask", "{server} に変更の有無をたずねています…"),
    msg(
        "fetch.step.download",
        "新しい状態をダウンロードしています（{size}）…",
    ),
    msg("fetch.step.import", "受信したコピーとして取り込んでいます…"),
    msg(
        "fetch.unchanged",
        "前回の取得から、サーバーに変更はありません。",
    ),
    msg(
        "fetch.done",
        "サーバーの状態を取得しました。ブランチとタグ {refs} 件を、受信したコピーとして取り込みました。あなた自身のブランチには触れていません。",
    ),
    msg(
        "fetch.done_next",
        "次の手順：届いたものをレビューして、あなた自身で prikk を使ってマージしてください。",
    ),
    msg(
        "fetch.not_a_repository",
        "「{dir}」は planeter-client clone で作られたリポジトリではないため、比べるものがありません。先に clone を実行してください。",
    ),
    msg(
        "fetch.split_view",
        "サーバーは今、{ref} を、前に見せたものから続かない状態で示しています。同じ名前で異なる 2 つの履歴が提供されているため、何も取り込みませんでした。メンテナーに伝えてください。",
    ),
    msg(
        "fetch.older_state",
        "サーバーは今、{ref} を更新 {shown} として示していますが、前は更新 {seen} を示していました。サーバーが過去に戻ることはありません。同じ名前で別のサーバーが応答している可能性があります。何もダウンロードしていません。メンテナーに伝えてください。",
    ),
    msg(
        "fetch.same_number",
        "サーバーは {ref} を、前と同じ更新 {seq} として示していますが、別の状態です。同じ名前で異なる 2 つの履歴が提供されています。何もダウンロードしていません。メンテナーに伝えてください。",
    ),
    msg(
        "fetch.no_sequence",
        "このサーバーは、ブランチが何回更新されたかを示しません（このプログラムが想定するより古いサーバーです）。そのため、古い状態と新しい状態を番号では区別できません。代わりに、このプログラムは各ブランチが何から続いているかを比べます。",
    ),
    msg(
        "prikk.missing",
        "prikk が見つかりません。prikk {floor} 以降をインストールし、PATH に含めてください。",
    ),
    msg(
        "prikk.too_old",
        "お使いの prikk は {found} ですが、このプログラムには {floor} 以降が必要です。prikk を更新してください。",
    ),
    msg(
        "prikk.cannot_start",
        "prikk は見つかりましたが、起動できませんでした：{reason}。prikk だけで動くか確かめてください：prikk --version",
    ),
    msg(
        "prikk.no_version",
        "prikk がバージョンを答えませんでした（答えたのは「{said}」）。prikk だけで動くか確かめてください：prikk --version",
    ),
    msg(
        "prikk.failed",
        "お使いの prikk で「{step}」ができませんでした：{reason}",
    ),
    msg(
        "local.file",
        "ローカルのファイルを使えませんでした：{path}（{reason}）。",
    ),
    msg(
        "local.state",
        "このプログラム自身のサーバー記録を読み書きできませんでした（{path}）。ファイルを削除すると、最初からやり直します。",
    ),
    msg(
        "progress.downloaded",
        "…{total} のうち {size} をダウンロードしました",
    ),
    msg("progress.sent", "…{total} のうち {size} を送りました"),
];
