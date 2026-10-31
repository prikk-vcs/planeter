//! English: the messages of `planeter-client`. Plain words (`docs/glossary.md`); one sentence for the reason and one for what to
//! do. Placeholders are `{name}`.

use super::{Entry, msg};

pub static CATALOG: &[Entry] = &[
    // ---- usage and help
    msg(
        "help.main",
        "planeter-client works with a planeter server using your own prikk and your own keys. It never holds a key and never changes a branch on the server.\n\nUsage:\n  planeter-client propose <https://host/owner/name> [--ref heads/NAME] [--proposal ID] [--title TITLE] [--description-file FILE] [--dry-run] [--retry]\n  planeter-client clone <https://host/owner/name> [DIR] [--trust-key KEY]... [--no-adopt]\n  planeter-client fetch [DIR]\n\nFor every command:\n  --token-file FILE   read the token from FILE (or set PLANETER_TOKEN); a token is never taken from the command line\n  --cacert FILE       trust only the certificate authorities in FILE; this replaces the built-in list\n  --lang en|ja        the language of the messages\n  --verbose           also show the raw reasons behind a message\n  --help, --version\n\nExit codes: 0 done, 1 the server refused, 2 wrong command line, 3 network or TLS, 4 your prikk failed, 5 local files or keys.\n\nExample:\n  planeter-client propose https://forge.example/alice/app --title \"Fix the parser\" --dry-run",
    ),
    msg(
        "help.propose",
        "planeter-client propose <https://host/owner/name> [options]\n\nBuilds your work against what the server already holds, with your own prikk, and sends it as a proposal. Nothing on the server is signed or merged: the work is held for review.\n\n  --ref heads/NAME          the branch of your repository to send (default heads/main)\n  --proposal ID             add a revision to this proposal instead of opening a new one\n  --title TITLE             the title of a new proposal (required for a new one)\n  --description-file FILE   the description, from FILE\n  --dry-run                 build and say what would be sent; send nothing\n  --retry                   send the same work again after the server said it did not finish\n  --token-file FILE, --cacert FILE, --lang, --verbose\n\nRun it inside your prikk repository.\n\nExample:\n  planeter-client propose https://forge.example/alice/app --title \"Fix the parser\"",
    ),
    msg(
        "help.clone",
        "planeter-client clone <https://host/owner/name> [DIR] [options]\n\nDownloads the repository, checks it with your own prikk, and shows you the maintainer keys it carries. Only the keys you type are trusted.\n\n  DIR                 where to put it (default: the repository's name)\n  --trust-key KEY     trust this whole public key; give it once per key when there is no terminal to type at\n  --no-adopt          keep everything as received copies and trust nothing\n  --token-file FILE, --cacert FILE, --lang, --verbose\n\nExample:\n  planeter-client clone https://forge.example/alice/app",
    ),
    msg(
        "help.fetch",
        "planeter-client fetch [DIR] [options]\n\nAsks the server what changed, downloads it if it did, checks it with your own prikk and lands it as received copies. It never changes your branches.\n\n  DIR   the repository (default: the current directory)\n  --token-file FILE, --cacert FILE, --lang, --verbose\n\nExample:\n  planeter-client fetch",
    ),
    msg(
        "version.line",
        "planeter-client {version} (build {commit}); it needs prikk {floor} or later",
    ),
    msg(
        "usage.unknown_command",
        "There is no command called “{name}”. Run planeter-client --help to see the commands.",
    ),
    msg(
        "usage.unknown_option",
        "“{name}” is not an option of this command. Run planeter-client {command} --help to see them.",
    ),
    msg(
        "usage.missing_value",
        "The option {name} needs a value. Run planeter-client {command} --help for an example.",
    ),
    msg(
        "usage.missing_argument",
        "The command needs {what}. Run planeter-client {command} --help for an example.",
    ),
    msg(
        "usage.too_many",
        "There is more on the command line than this command takes (“{extra}”). Run planeter-client {command} --help.",
    ),
    msg(
        "usage.bad_lang",
        "“{value}” is not a language this program has. Use en or ja.",
    ),
    msg(
        "usage.bad_url",
        "“{url}” is not the address of a repository. It looks like https://host/owner/name.",
    ),
    msg(
        "usage.plain_http",
        "“{url}” starts with http://, and this program only talks to servers over https. Use an https:// address.",
    ),
    msg(
        "usage.no_repository",
        "“{dir}” is not a prikk repository. Run this command inside your own repository, or name it.",
    ),
    // ---- the token
    msg(
        "token.required",
        "Proposing needs a token, and none was given. Set PLANETER_TOKEN, or pass --token-file FILE.",
    ),
    msg(
        "token.file_unreadable",
        "The token file {path} could not be read. Check the path and who may read it.",
    ),
    msg(
        "token.file_too_open",
        "The token file {path} can be read by other users, so it is not used. Run chmod 600 on it, then try again.",
    ),
    msg(
        "token.file_empty",
        "The token file {path} is empty. Put the token in it, on one line.",
    ),
    // ---- the network and TLS
    msg(
        "tls.unknown_issuer",
        "This server's certificate is not signed by a certificate authority this program trusts. If you know the server uses its own, pass --cacert FILE with that authority's certificate.",
    ),
    msg(
        "tls.wrong_name",
        "This server's certificate is for another name than the one in the address. Check the address, or ask the person who runs the server.",
    ),
    msg(
        "tls.expired",
        "This server's certificate has expired (or is not valid yet). Tell the person who runs the server; check your computer's clock too.",
    ),
    msg(
        "tls.other",
        "The secure connection to the server could not be made. Run it again with --verbose to see the reason, and tell the person who runs the server.",
    ),
    msg(
        "net.unreachable",
        "The server at {host} could not be reached. Check the address and your connection, then try again.",
    ),
    msg(
        "net.timeout",
        "The server at {host} did not answer in time. Try again in a little while.",
    ),
    msg(
        "net.cut_short",
        "The connection ended before the whole answer arrived. Run the same command again to continue.",
    ),
    msg(
        "net.redirect",
        "The server answered with a redirect to {location}, and this program never follows one. Check the address you gave.",
    ),
    msg(
        "net.too_long",
        "The server sent more than it said it would, so the answer was dropped. Try again; if it keeps happening, tell the person who runs the server.",
    ),
    msg(
        "net.other",
        "The request to the server failed. Run it again with --verbose to see the reason.",
    ),
    msg(
        "net.bad_answer",
        "The server's answer could not be understood. Check that the address is a planeter server, and try again.",
    ),
    msg(
        "net.cacert_unreadable",
        "The certificate file {path} could not be read as certificates. Pass a PEM file with the authority's certificate.",
    ),
    msg(
        "net.proxy",
        "The proxy named in HTTPS_PROXY could not be used. Check the variable, or set NO_PROXY for this server.",
    ),
    // ---- what the server said
    msg("server.refused", "The server refused: {message} {hint}"),
    msg(
        "server.refused_plain",
        "The server refused the request ({status}) and gave no reason. Try again, and tell the person who runs the server if it keeps happening.",
    ),
    msg(
        "server.not_found",
        "The server has no such repository, or you may not read it. Check the address and that your token is the right one.",
    ),
    msg(
        "server.forbidden",
        "You may not do this on this repository. Ask a maintainer to give you the right to propose.",
    ),
    msg(
        "server.sign_in",
        "The server did not accept the token. Check that it is current and that it was made for this server.",
    ),
    msg(
        "server.rate_limited",
        "The server is asked too often from here. Wait {seconds} seconds, then try again.",
    ),
    msg(
        "server.busy",
        "The server is busy. Wait {seconds} seconds, then run the same command again.",
    ),
    msg(
        "server.off",
        "This server is not set up to hand out or take in repositories. Tell the person who runs it.",
    ),
    msg(
        "server.too_large",
        "The server will not take this much at once. Send a smaller piece of work, or ask the person who runs the server to raise its limit.",
    ),
    msg(
        "server.unavailable",
        "The server cannot do this right now. Try again in a little while.",
    ),
    // ---- propose
    msg(
        "propose.step.advertise",
        "Asking {server} what it already holds…",
    ),
    msg(
        "propose.step.have",
        "Fetching what the server holds of {ref}…",
    ),
    msg(
        "propose.step.build",
        "Building your work against it with your own prikk…",
    ),
    msg(
        "propose.step.declare",
        "{count} patches are new, on top of {base}.",
    ),
    msg("propose.step.send", "Sending {size}…"),
    msg(
        "propose.done_new",
        "Proposed as proposal #{id} (revision {revision}). Your work is held for review on this server; no branch has changed. Reviewers can see it at {url}.",
    ),
    msg(
        "propose.done_revision",
        "Added as revision {revision} of proposal #{id}. Your work is held for review on this server; no branch has changed. Reviewers can see it at {url}.",
    ),
    msg(
        "propose.already",
        "This work was already recorded as revision {revision} of proposal #{id}, so nothing new was added. Reviewers can see it at {url}.",
    ),
    msg(
        "propose.nothing_new",
        "The server already holds everything you have on {ref}, so there is nothing to propose.",
    ),
    msg(
        "propose.dry_run",
        "Dry run: nothing was sent. This would propose {count} patches ({size}) on top of {base} to {url}.",
    ),
    msg("propose.dry_run_details", "The patches are: {patches}."),
    msg(
        "propose.no_have",
        "The server offers no list of what it holds for {ref}, so your work cannot be built against it. Check the branch name; only a branch of the server has one.",
    ),
    msg(
        "propose.branch_gone",
        "Your repository has no branch {ref}. Name the branch you want to send with --ref.",
    ),
    msg(
        "propose.base_not_in_history",
        "The server's {ref} is at a point your own {ref} does not contain. Fetch the server's state, bring your work on top of it yourself, then propose again.",
    ),
    msg(
        "propose.over_bound",
        "Your work is {size}, and this server takes at most {limit} at once. Send it in smaller pieces, or ask the person who runs the server to raise the limit.",
    ),
    msg(
        "propose.too_many_objects",
        "Your work has {count} new patches, and this server takes at most {limit} at once. Send it in smaller pieces, or ask the person who runs the server to raise the limit.",
    ),
    msg(
        "propose.title_needed",
        "A new proposal needs a title. Add --title \"…\", or add --proposal ID to extend one that exists.",
    ),
    msg(
        "propose.incomplete",
        "The server did not finish taking your work in. Send the same work again once the repository has recovered.",
    ),
    msg(
        "propose.incomplete_retry",
        "Run the same command with --retry to send it again.",
    ),
    msg(
        "propose.quota",
        "You have reached a limit on this server: {limit} {unit} for {quota}, and {used} are used.",
    ),
    msg(
        "propose.quota_remedy",
        "Ask a maintainer to close finished proposals, or to raise the limit.",
    ),
    msg(
        "propose.needs_recovery",
        "The server is repairing this repository and takes no new work until it is done. Try again in a little while.",
    ),
    msg(
        "propose.declaration",
        "The list of patches this program declared does not match what the artifact holds. Run the command again; if it keeps happening, tell the person who runs the server.",
    ),
    msg(
        "propose.base_unknown",
        "The server does not know the point your work was built on. Fetch the server's state and build on top of it, then propose again.",
    ),
    msg(
        "propose.too_large_for_server",
        "The server cannot take work of this size right now. Send a smaller piece, or try again in a little while.",
    ),
    msg(
        "propose.not_open",
        "That proposal is no longer open, so it takes no new revision. Open a new proposal with --title.",
    ),
    msg(
        "propose.not_yours",
        "That proposal belongs to someone else, and you may not add to it. Open your own with --title.",
    ),
    msg(
        "propose.unknown_proposal",
        "The server has no proposal with that id on this repository. Check the id.",
    ),
    msg(
        "propose.no_patches",
        "Nothing on {ref} is new to the server. There is nothing to propose.",
    ),
    msg(
        "propose.accept_refused",
        "The server looked at your work and did not accept it: {reason}",
    ),
    // ---- clone
    msg(
        "clone.step.download",
        "Downloading the repository from {server}…",
    ),
    msg("clone.step.verify", "Checking it with your own prikk…"),
    msg("clone.step.import", "Landing it in {dir}…"),
    msg(
        "clone.keys.header",
        "This repository carries {count} maintainer keys. Only the keys you type here are trusted; a key you skip trusts nobody.",
    ),
    msg(
        "clone.keys.none",
        "This repository carries no maintainer key, so there is nothing to trust.",
    ),
    msg("clone.key.show", "Key {number} of {count}:\n  {key}"),
    msg(
        "clone.key.ask",
        "To trust key {number}, type the whole key (the groups above, with or without the spaces). To skip it, press Enter:",
    ),
    msg("clone.key.trusted", "Key {number} is trusted."),
    msg(
        "clone.key.skipped",
        "Key {number} is skipped; nothing it signed is trusted.",
    ),
    msg(
        "clone.key.wrong",
        "That is not key {number}. Try again, or press Enter to skip it.",
    ),
    msg(
        "clone.key.not_carried",
        "The key you gave with --trust-key is not one this repository carries ({key}). Nothing was downloaded into your repository; check the key with the maintainer.",
    ),
    msg(
        "clone.no_terminal",
        "There is no terminal to type the keys at, and no --trust-key was given. Nothing was cloned. Pass --trust-key KEY once for each key you trust, or --no-adopt to trust nothing.",
    ),
    msg(
        "clone.no_adopt",
        "Everything was kept as received copies. Nothing is verified or trusted.",
    ),
    msg(
        "clone.done",
        "Cloned {name} into {dir}. {trusted} of {count} keys are trusted.",
    ),
    msg(
        "clone.done_next",
        "Next: go into {dir}, look at it with prikk, and run planeter-client fetch later to see what changes.",
    ),
    msg(
        "clone.dir_exists",
        "{dir} already exists and is not empty. Name another directory, or remove it first.",
    ),
    msg(
        "clone.verify_failed",
        "Your prikk checked the downloaded repository and it did not pass. Nothing was put in {dir}. Tell the person who runs the server.",
    ),
    msg(
        "clone.archive_bad",
        "The downloaded file is not a repository archive your prikk can read. Nothing was put in {dir}.",
    ),
    // ---- fetch
    msg("fetch.step.ask", "Asking {server} what changed…"),
    msg("fetch.step.download", "Downloading the new state ({size})…"),
    msg("fetch.step.import", "Landing it as received copies…"),
    msg(
        "fetch.unchanged",
        "Nothing changed on the server since the last fetch.",
    ),
    msg(
        "fetch.done",
        "Fetched the server's state: {refs} branches and tags came as received copies. Your own branches were not touched.",
    ),
    msg(
        "fetch.done_next",
        "Next: review what came, then merge it yourself with prikk.",
    ),
    msg(
        "fetch.not_a_repository",
        "“{dir}” is not a repository made by planeter-client clone, so there is nothing to compare with. Run clone first.",
    ),
    msg(
        "fetch.split_view",
        "The server now shows {ref} at a state that does not continue from the one it showed you before. Two different histories are being served under one name, so nothing was landed. Tell the maintainers.",
    ),
    msg(
        "fetch.no_sequence",
        "This server does not say how many updates a branch has had (it is older than this program expects), so an older state cannot be told from a newer one by number; this program compares what each branch continues from instead.",
    ),
    msg(
        "fetch.older_state",
        "The server now shows {ref} at update {shown}, but it showed update {seen} before. A server does not go back in time; two different servers may be answering under one name. Nothing was downloaded. Tell the maintainers.",
    ),
    msg(
        "fetch.same_number",
        "The server shows {ref} at update {seq}, as it did before, but as a different state. Two different histories are being served under one name. Nothing was downloaded. Tell the maintainers.",
    ),
    // ---- prikk and local state
    msg(
        "prikk.missing",
        "prikk was not found. Install prikk {floor} or later, and make sure it is on your PATH.",
    ),
    msg(
        "prikk.too_old",
        "Your prikk is {found}, and this program needs {floor} or later. Update prikk.",
    ),
    msg(
        "prikk.cannot_start",
        "prikk was found but could not be started: {reason}. Check that it runs by itself: prikk --version.",
    ),
    msg(
        "prikk.no_version",
        "prikk did not say which version it is (it said: “{said}”). Check that it runs by itself: prikk --version.",
    ),
    msg("prikk.failed", "Your prikk could not do “{step}”: {reason}"),
    msg(
        "local.file",
        "A local file could not be used: {path} ({reason}).",
    ),
    msg(
        "local.state",
        "This program's own record of the server could not be read or written ({path}). Remove the file to start it afresh.",
    ),
    // ---- progress
    msg("progress.downloaded", "…{size} of {total} downloaded"),
    msg("progress.sent", "…{size} of {total} sent"),
];
