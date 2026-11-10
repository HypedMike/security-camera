//! Which Telegram users may open the mini app.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::util::lock;

/// Tells whether a Telegram user is in the configured chat.
pub trait Members: Send + Sync {
    fn is_member(&self, user_id: i64) -> Result<bool, String>;
}

// How long an answer is remembered. Asking Telegram on every request would be
// slow and would let anyone who opens the app make the program call Telegram
// as often as they like. A yes lasts longer than a no, so somebody who has
// just joined the group gets in soon, and somebody who left is out in minutes.
// A failed check is only remembered briefly: it says nothing about the user.
pub(crate) const YES_FOR: Duration = Duration::from_secs(10 * 60);
pub(crate) const NO_FOR: Duration = Duration::from_secs(60);
pub(crate) const ERROR_FOR: Duration = Duration::from_secs(10);

/// The cache is cleaned when it has this many answers.
const CACHE_LIMIT: usize = 1000;

struct Verdict {
    ok: bool,
    until: Instant,
}

type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// Decides which Telegram users may open the mini app: those who are in the
/// chat the bot is configured for (TELEGRAM_CHAT_ID).
///
/// - private chat: only its user. A private chat's id is the id of the user.
/// - group: whoever is in the group, asked to Telegram when they open the app
///   (Telegram can't list a group's members) and remembered for a while.
pub struct Access {
    /// the user of a private chat, 0 for a group
    owner: i64,
    members: Option<Arc<dyn Members>>,
    now: Clock,
    cache: Mutex<HashMap<i64, Verdict>>,
}

impl Access {
    /// The Access for the chat id. `members` is only used when the chat is a
    /// group.
    pub fn new(chat_id: &str, members: Option<Arc<dyn Members>>) -> Result<Access, String> {
        Access::with_clock(chat_id, members, Arc::new(Instant::now))
    }

    pub(crate) fn with_clock(
        chat_id: &str,
        members: Option<Arc<dyn Members>>,
        now: Clock,
    ) -> Result<Access, String> {
        let id: i64 = match chat_id.trim().parse() {
            Ok(id) if id != 0 => id,
            _ => return Err("TELEGRAM_CHAT_ID is not a chat id".into()),
        };
        if id < 0 && members.is_none() {
            return Err("a group chat needs a way to check who is in it".into());
        }
        Ok(Access {
            owner: if id > 0 { id } else { 0 },
            members,
            now,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Whether the Telegram user may open the mini app.
    pub fn allowed(&self, user_id: i64) -> bool {
        if self.owner != 0 {
            return user_id == self.owner;
        }
        let now = (self.now)();
        if let Some(v) = lock(&self.cache).get(&user_id)
            && now < v.until
        {
            return v.ok;
        }
        let members = self
            .members
            .as_ref()
            .expect("a group has a way to check it");
        let verdict = match members.is_member(user_id) {
            // closed: when in doubt, nobody gets the camera
            Err(e) => {
                eprintln!("mini app: can't check whether user {user_id} is in the group: {e}");
                Verdict {
                    ok: false,
                    until: now + ERROR_FOR,
                }
            }
            Ok(true) => Verdict {
                ok: true,
                until: now + YES_FOR,
            },
            Ok(false) => Verdict {
                ok: false,
                until: now + NO_FOR,
            },
        };
        let ok = verdict.ok;
        let mut cache = lock(&self.cache);
        if cache.len() >= CACHE_LIMIT {
            cache.retain(|_, old| now < old.until);
        }
        cache.insert(user_id, verdict);
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        members: Mutex<HashMap<i64, bool>>,
        error: Mutex<Option<String>>,
        calls: AtomicUsize,
    }

    impl Members for Fake {
        fn is_member(&self, id: i64) -> Result<bool, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(e) = &*self.error.lock().unwrap() {
                return Err(e.clone());
            }
            Ok(self
                .members
                .lock()
                .unwrap()
                .get(&id)
                .copied()
                .unwrap_or(false))
        }
    }

    fn fake(members: &[i64]) -> Arc<Fake> {
        Arc::new(Fake {
            members: Mutex::new(members.iter().map(|&m| (m, true)).collect()),
            error: Mutex::new(None),
            calls: AtomicUsize::new(0),
        })
    }

    /// An Access whose clock only moves when the test says so.
    fn group(m: &Arc<Fake>) -> (Access, Arc<Mutex<Instant>>) {
        let clock = Arc::new(Mutex::new(Instant::now()));
        let c = clock.clone();
        let a = Access::with_clock(
            "-100123",
            Some(m.clone()),
            Arc::new(move || *c.lock().unwrap()),
        )
        .unwrap();
        (a, clock)
    }

    fn advance(clock: &Mutex<Instant>, d: Duration) {
        *clock.lock().unwrap() += d;
    }

    #[test]
    fn private_chat_lets_only_its_user_in() {
        let a = Access::new("42", None).unwrap();
        assert!(a.allowed(42) && !a.allowed(43));
    }

    #[test]
    fn group_members_get_in_and_answers_are_remembered() {
        let m = fake(&[7]);
        let (a, clock) = group(&m);
        assert!(a.allowed(7) && !a.allowed(8), "members in, others out");
        a.allowed(7);
        a.allowed(8);
        assert_eq!(
            m.calls.load(Ordering::SeqCst),
            2,
            "answers are not remembered"
        );

        // a no is forgotten soon: somebody who just joined gets in
        m.members.lock().unwrap().insert(8, true);
        advance(&clock, NO_FOR + Duration::from_secs(1));
        assert!(
            a.allowed(8),
            "a new member is still refused after the no expired"
        );
        // a yes lasts longer, then somebody who left is out
        m.members.lock().unwrap().remove(&7);
        assert!(a.allowed(7), "a yes was not kept");
        advance(&clock, YES_FOR + Duration::from_secs(1));
        assert!(!a.allowed(7), "somebody who left is still let in");
    }

    #[test]
    fn group_fails_closed_and_forgets_the_failure() {
        let m = fake(&[7]);
        *m.error.lock().unwrap() = Some("down".into());
        let (a, clock) = group(&m);
        assert!(!a.allowed(7), "let in with the check failing");
        assert!(!a.allowed(7));
        assert_eq!(
            m.calls.load(Ordering::SeqCst),
            1,
            "the failure is remembered for a moment"
        );
        *m.error.lock().unwrap() = None;
        advance(&clock, ERROR_FOR + Duration::from_secs(1));
        assert!(
            a.allowed(7),
            "still refused after the failure was forgotten"
        );
    }

    #[test]
    fn the_cache_does_not_grow_forever() {
        let m = fake(&[]);
        let (a, clock) = group(&m);
        for id in 1..=CACHE_LIMIT as i64 {
            a.allowed(id);
        }
        advance(&clock, YES_FOR + Duration::from_secs(1));
        a.allowed(99_999);
        assert_eq!(lock(&a.cache).len(), 1);
    }

    #[test]
    fn bad_chat_ids_are_refused() {
        for id in ["", "abc", "0"] {
            assert!(Access::new(id, Some(fake(&[]))).is_err(), "{id:?}");
        }
        assert!(
            Access::new("-100123", None).is_err(),
            "a group with no way to check it"
        );
    }
}
