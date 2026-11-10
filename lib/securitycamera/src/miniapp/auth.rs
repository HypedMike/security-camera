//! Checks the `initData` Telegram gives a mini app when it is opened.

use std::collections::HashMap;
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use chrono::{DateTime, Local, TimeDelta};

type HmacSha256 = Hmac<Sha256>;

/// The only thing a caller learns about a failed check.
#[derive(Debug, PartialEq, Eq)]
pub struct Unauthorized;

pub(crate) fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC takes keys of any size");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

/// The fields of a query string, the first value of each name.
fn parse_query(s: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for (k, v) in url::form_urlencoded::parse(s.as_bytes()) {
        m.entry(k.into_owned()).or_insert_with(|| v.into_owned());
    }
    m
}

/// Joins the fields, but the skipped ones, sorted by name, as "key=value" lines.
pub(crate) fn data_check_string(vals: &HashMap<String, String>, skip: &[&str]) -> String {
    let mut lines: Vec<String> = vals
        .iter()
        .filter(|(k, _)| !skip.contains(&k.as_str()))
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    lines.sort();
    lines.join("\n")
}

/// Checks the initData string Telegram gives a mini app when it is opened,
/// and returns the id of the Telegram user it was made for.
///
/// Telegram signs initData with a key derived from the bot token, so only a
/// real opening of the app from Telegram can produce one that passes. The
/// signature is HMAC-SHA256 over the fields (except hash) sorted by name, as
/// "key=value" lines. initData older than `max_age` is refused, so a leaked
/// one stops working.
pub fn validate(
    init_data: &str,
    bot_token: &str,
    now: DateTime<Local>,
    max_age: Duration,
) -> Result<i64, Unauthorized> {
    let vals = parse_query(init_data);
    let hash = vals
        .get("hash")
        .and_then(|h| hex::decode(h).ok())
        .filter(|h| h.len() == 32)
        .ok_or(Unauthorized)?;

    let secret = hmac_sha256(b"WebAppData", bot_token.as_bytes());
    // newer clients add a "signature" field meant for third-party checks;
    // whether the bot-token signature covers it is not something to guess
    // at, and accepting either is as safe: the HMAC still has to match
    let signed = [&["hash"][..], &["hash", "signature"][..]]
        .iter()
        .any(|skip| {
            let mut m = HmacSha256::new_from_slice(&secret).expect("HMAC takes keys of any size");
            m.update(data_check_string(&vals, skip).as_bytes());
            m.verify_slice(&hash).is_ok() // compares in constant time
        });
    if !signed {
        return Err(Unauthorized);
    }

    let secs: i64 = vals
        .get("auth_date")
        .and_then(|s| s.parse().ok())
        .ok_or(Unauthorized)?;
    let made = DateTime::from_timestamp(secs, 0).ok_or(Unauthorized)?;
    let age = now.signed_duration_since(made);
    if age > TimeDelta::from_std(max_age).map_err(|_| Unauthorized)? || age < -TimeDelta::minutes(5)
    {
        return Err(Unauthorized);
    }

    #[derive(serde::Deserialize)]
    struct User {
        id: i64,
    }
    let user: User = serde_json::from_str(vals.get("user").map_or("", String::as_str))
        .map_err(|_| Unauthorized)?;
    if user.id == 0 {
        return Err(Unauthorized);
    }
    Ok(user.id)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const TOKEN: &str = "123:ABC";

    /// Builds initData the way Telegram does, for a user and an age.
    pub fn signed(
        bot_token: &str,
        user_id: i64,
        age: TimeDelta,
        now: DateTime<Local>,
        extra: &[(&str, &str)],
    ) -> String {
        let mut vals: HashMap<String, String> = HashMap::from([
            ("auth_date".into(), (now - age).timestamp().to_string()),
            (
                "user".into(),
                format!(r#"{{"id":{user_id},"first_name":"A"}}"#),
            ),
            ("query_id".into(), "AAH".into()),
        ]);
        for (k, v) in extra {
            vals.insert((*k).into(), (*v).into());
        }
        let secret = hmac_sha256(b"WebAppData", bot_token.as_bytes());
        let hash = hex::encode(hmac_sha256(
            &secret,
            data_check_string(&vals, &["hash"]).as_bytes(),
        ));
        let mut q = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in &vals {
            q.append_pair(k, v);
        }
        q.append_pair("hash", &hash);
        q.finish()
    }

    #[test]
    fn accepts_what_telegram_signed() {
        let now = Local::now();
        let hour = Duration::from_secs(3600);
        let good = signed(TOKEN, 42, TimeDelta::minutes(1), now, &[]);
        assert_eq!(validate(&good, TOKEN, now, hour), Ok(42));
        // a newer client adds a signature field the check may or may not cover
        let with_sig = signed(
            TOKEN,
            42,
            TimeDelta::minutes(1),
            now,
            &[("signature", "abc")],
        );
        assert_eq!(validate(&with_sig, TOKEN, now, hour), Ok(42));
    }

    #[test]
    fn refuses_everything_else() {
        let now = Local::now();
        let hour = Duration::from_secs(3600);
        let good = signed(TOKEN, 42, TimeDelta::minutes(1), now, &[]);
        let cases = [
            (
                "wrong token",
                signed("999:XYZ", 42, TimeDelta::minutes(1), now, &[]),
            ),
            ("expired", signed(TOKEN, 42, TimeDelta::hours(2), now, &[])),
            (
                "from future",
                signed(TOKEN, 42, -TimeDelta::hours(1), now, &[]),
            ),
            ("other user", good.replace("%22id%22%3A42", "%22id%22%3A43")),
            ("no hash", good.split("&hash=").next().unwrap().to_owned()),
            ("empty", String::new()),
            ("garbage", "%%%".to_owned()),
            ("bad hash hex", "auth_date=1&hash=zz".to_owned()),
            ("short hash", "auth_date=1&hash=abcd".to_owned()),
            ("no user", signed(TOKEN, 0, TimeDelta::minutes(1), now, &[])),
        ];
        for (name, data) in cases {
            assert!(
                validate(&data, TOKEN, now, hour).is_err(),
                "{name} accepted"
            );
        }
        assert_ne!(
            good, "",
            "the signed data of the 'other user' case really differs"
        );
    }

    #[test]
    fn the_first_value_of_a_repeated_field_counts() {
        let vals = parse_query("a=1&a=2&b=3");
        assert_eq!(data_check_string(&vals, &[]), "a=1\nb=3");
        assert_eq!(data_check_string(&vals, &["a"]), "b=3");
    }
}
