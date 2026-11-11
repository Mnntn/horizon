//! Opt-in test observations after successful NETWORK preparation, before publication.
use std::sync::Mutex;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub work: usize,
    pub replies: usize,
}
static WATCH: Mutex<Option<(String, Counts)>> = Mutex::new(None);
pub struct Watch;
impl Watch {
    pub fn begin(channel: &str) -> Self {
        let mut watch = WATCH.lock().unwrap();
        assert!(watch.is_none());
        *watch = Some((channel.into(), Counts::default()));
        Self
    }
    pub fn snapshot(&self) -> Counts {
        WATCH.lock().unwrap().as_ref().unwrap().1
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        *WATCH.lock().unwrap() = None;
    }
}
pub(super) fn prepared(channel: &str, work: bool) {
    if let Some((watched, counts)) = WATCH.lock().unwrap().as_mut()
        && watched == channel
    {
        if work {
            counts.work += 1;
        } else {
            counts.replies += 1;
        }
    }
}
