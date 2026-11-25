//! The two settings that are about the sign-in provider and how long a sign-in lasts (RFC 014 D-11, D-13), read once at start-up and judged
//! **before anything is opened**: `PLANETER_OIDC_ALLOW_PRIVATE_ISSUER` and `PLANETER_SESSION_HOURS`. A wrong value refuses the start with a
//! sentence (never clamped, never ignored); a right one gives the start-up lines (counts, a host and a number: no secret).

use planeter_web::catalog::{Key, T};

/// What the environment asked for.
#[derive(Debug, PartialEq, Eq)]
pub struct ProviderSettings {
    /// The one host that may be on a private address: the issuer's host, when the operator said so.
    pub private_host: Option<String>,
    /// The lifetime of a new sign-in, in hours.
    pub session_hours: u64,
}

/// Read the settings from `env` (a lookup of variables). `Err` is the sentence to print before refusing to start.
pub fn read(env: &dyn Fn(&str) -> Option<String>, t: &T) -> Result<ProviderSettings, String> {
    let issuer = env("PLANETER_OIDC_ISSUER").filter(|v| !v.is_empty());
    let allow = match env("PLANETER_OIDC_ALLOW_PRIVATE_ISSUER")
        .as_deref()
        .map(str::trim)
    {
        None | Some("") | Some("off") => false,
        Some("on") => true,
        Some(other) => {
            return Err(t
                .msg(Key::ProviderErrorAllowPrivateValue)
                .with("value", other)
                .text());
        }
    };
    let private_host = if allow {
        let Some(issuer) = issuer else {
            return Err(t.get(Key::ProviderErrorAllowPrivateNoProvider));
        };
        match planeter_core::url_host(&issuer) {
            Some(h) if !h.is_empty() => Some(h),
            _ => {
                return Err(t
                    .msg(Key::ProviderErrorIssuerHost)
                    .with("issuer", issuer)
                    .text());
            }
        }
    } else {
        None
    };
    let session_hours = planeter_auth::session_hours_from(env("PLANETER_SESSION_HOURS").as_deref())
        .map_err(|got| {
            t.msg(Key::ProviderErrorSessionHours)
                .with("min", planeter_auth::SESSION_HOURS_MIN)
                .with("max", planeter_auth::SESSION_HOURS_MAX)
                .with("value", got)
                .text()
        })?;
    Ok(ProviderSettings {
        private_host,
        session_hours,
    })
}

/// **The egress guard of the sign-in provider, and of nothing else**: its own instance, so that no other outbound path inherits the
/// allowance (a caller builds its own guard; this one is handed to the provider's fetcher only). With the operator's `on`, the issuer's host
/// (and only it) may be on a private address; every other host, and the scheme (`https` only), are judged as for any caller.
pub fn provider_guard(s: &ProviderSettings) -> planeter_core::StdEgressGuard {
    planeter_core::StdEgressGuard::new(planeter_core::EgressPolicy {
        allow_private_for_host: s.private_host.clone(),
        ..Default::default()
    })
}

/// The start-up lines for these settings (the allowance names its host; the lifetime its hours).
pub fn lines(s: &ProviderSettings, t: &T) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(host) = &s.private_host {
        out.push(
            t.msg(Key::ProviderLineAllowPrivate)
                .with("host", host)
                .text(),
        );
    }
    out.push(
        t.msg(Key::ProviderLineSessionHours)
            .n(s.session_hours)
            .text(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use planeter_web::catalog::{EN, Lang};

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    fn en() -> T {
        T::new(EN)
    }

    fn ja() -> T {
        T::new(Lang::from_tag("ja").unwrap())
    }

    #[test]
    fn the_provider_guard_admits_the_one_host_and_nothing_else_and_is_not_shared() {
        use planeter_core::{EgressError, EgressGuard};
        let on = ProviderSettings {
            private_host: Some("127.0.0.1".into()),
            session_hours: 168,
        };
        let g = provider_guard(&on);
        assert!(
            g.check("https://127.0.0.1:8443/x").is_ok(),
            "the issuer's own host"
        );
        for refused in [
            "https://127.0.0.2/x",
            "https://10.0.0.5/x",
            "https://169.254.169.254/x",
            "https://[::1]/x",
            "https://100.64.0.1/x",
        ] {
            assert!(
                matches!(g.check(refused), Err(EgressError::BlockedAddress(_))),
                "{refused}"
            );
        }
        assert!(
            matches!(
                g.check("http://127.0.0.1/x"),
                Err(EgressError::DisallowedScheme(_))
            ),
            "https only"
        );
        assert!(matches!(
            g.check("https://user:pw@127.0.0.1/x"),
            Err(EgressError::Unparsable(_))
        ));
        // With the allowance off, even the issuer's host is judged like any other.
        let off = ProviderSettings {
            private_host: None,
            session_hours: 168,
        };
        assert!(matches!(
            provider_guard(&off).check("https://127.0.0.1/x"),
            Err(EgressError::BlockedAddress(_))
        ));
        // A guard any other caller builds is not touched by it.
        assert!(matches!(
            planeter_core::StdEgressGuard::default().check("https://127.0.0.1:8443/x"),
            Err(EgressError::BlockedAddress(_))
        ));
    }

    #[test]
    fn the_defaults_are_off_and_seven_days() {
        let s = read(&env(&[]), &en()).unwrap();
        assert_eq!(
            s,
            ProviderSettings {
                private_host: None,
                session_hours: 168
            }
        );
        assert_eq!(lines(&s, &en()), vec!["Sign-ins last 168 hours."]);
        assert_eq!(
            lines(&s, &ja()),
            vec!["ログイン中の状態は 168 時間続きます。"]
        );
    }

    #[test]
    fn on_names_the_issuers_host_and_says_so() {
        let e = env(&[
            (
                "PLANETER_OIDC_ISSUER",
                "https://IDP.internal.example:8443/realm",
            ),
            ("PLANETER_OIDC_ALLOW_PRIVATE_ISSUER", "on"),
            ("PLANETER_SESSION_HOURS", "24"),
        ]);
        let s = read(&e, &en()).unwrap();
        assert_eq!(s.private_host.as_deref(), Some("idp.internal.example"));
        assert_eq!(s.session_hours, 24);
        let l = lines(&s, &en());
        assert!(
            l[0].contains("idp.internal.example") && l[0].contains("private address"),
            "{l:?}"
        );
        assert_eq!(l[1], "Sign-ins last 24 hours.");
        assert_eq!(
            lines(
                &ProviderSettings {
                    private_host: None,
                    session_hours: 1
                },
                &en()
            ),
            vec!["Sign-ins last 1 hour."]
        );
    }

    #[test]
    fn a_wrong_value_is_refused_with_a_sentence_in_both_languages() {
        let bad_flag = env(&[
            ("PLANETER_OIDC_ISSUER", "https://idp.example"),
            ("PLANETER_OIDC_ALLOW_PRIVATE_ISSUER", "yes"),
        ]);
        assert!(
            read(&bad_flag, &en())
                .unwrap_err()
                .contains("must be on or off, and it is yes")
        );
        assert!(
            read(&bad_flag, &ja())
                .unwrap_err()
                .contains("今の値は yes です")
        );
        let none = env(&[("PLANETER_OIDC_ALLOW_PRIVATE_ISSUER", "on")]);
        assert!(
            read(&none, &en())
                .unwrap_err()
                .contains("needs a sign-in provider")
        );
        assert!(
            read(&none, &ja())
                .unwrap_err()
                .contains("プロバイダーが必要です")
        );
        let nohost = env(&[
            ("PLANETER_OIDC_ISSUER", "not a url"),
            ("PLANETER_OIDC_ALLOW_PRIVATE_ISSUER", "on"),
        ]);
        assert!(
            read(&nohost, &en())
                .unwrap_err()
                .contains("has no host that can be allowed")
        );
        for v in ["0", "721", "x", "1.5", "-1"] {
            let e = move |k: &str| (k == "PLANETER_SESSION_HOURS").then(|| v.to_owned());
            let err = read(&e, &en()).unwrap_err();
            assert!(
                err.contains("from 1 to 720") && err.contains(&format!("it is {v}")),
                "{err}"
            );
            assert!(read(&e, &ja()).unwrap_err().contains("1 から 720 まで"));
        }
    }
}
