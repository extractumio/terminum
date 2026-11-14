// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-38: input and capture submission order, regardless of when their replies are polled.

use super::*;
use crate::page::{trace::Recorder, Pages};
use crate::tmux::control::Control;

fn page() -> (Arc<Page>, tokio::sync::mpsc::Receiver<String>) {
    let recorder = Recorder::new(crate::testutil::temp_root().join("trace-not-written"));
    let (page, rx) = Pages::default().add(Default::default(), String::new(), recorder).unwrap();
    page.tracer.lock().handle(&json!({"stages": true}), "test");
    (page, rx)
}

#[tokio::test]
async fn a_key_is_in_its_echo_frame_before_its_reply_is_polled() {
    let (page, _rx) = page();
    let (client, mut commands) = Control::test_queue();
    let mut key: Vec<_> = [77, 78].into_iter().filter_map(|k| arrived(&page, Some(k))).collect();
    let input = submit(&page, &client, &mut key, vec!["send-keys -t %1 -l zz".into()]);
    let (began, screen) = capture(&page, &client, vec!["capture-pane -p -t %1".into()]);
    let (line, input_reply, _) = commands.try_recv().expect("input queued without polling");
    assert!(line.starts_with("send-keys"));
    let (line, screen_reply, _) = commands.try_recv().expect("capture queued without polling");
    assert!(line.starts_with("capture-pane"));
    screen_reply.send(Ok(vec!["z".into()])).unwrap();
    assert_eq!(screen.await, vec![Ok(vec!["z".into()])]);
    captured(&page, began);
    let got = frame(&page, true).expect("the echoed key, even while input's reply is pending");
    assert_eq!(got["tr"]["ks"].as_array().unwrap().len(), 2, "queued repeats share one submission");
    assert_eq!(got["tr"]["ks"][0]["k"], 77);
    assert_eq!(got["tr"]["ks"][1]["k"], 78);
    for stage in ["to_tmux_ms", "until_change_ms", "capture_ms", "encode_ms", "total_ms"] {
        assert!(got["tr"]["ks"][0][stage].is_number(), "{stage}: {got}");
    }
    input_reply.send(Ok(vec![])).unwrap();
    assert_eq!(input.await, vec![Ok(vec![])]);
    assert!(key.is_empty(), "a multi-part input registers only once");
    let more = submit(&page, &client, &mut key, vec!["send-keys -t %1 -l more".into()]);
    commands.try_recv().unwrap().1.send(Ok(vec![])).unwrap();
    assert!(more.await[0].is_ok());
    captured(&page, Instant::now());
    assert_eq!(frame(&page, true), None, "later parts do not duplicate the key");
}

#[tokio::test]
async fn a_capture_queued_before_input_cannot_answer_it() {
    let (page, _rx) = page();
    let (client, mut commands) = Control::test_queue();
    let (began, screen) = capture(&page, &client, vec!["capture-pane -p -t %1".into()]);
    let mut key = arrived(&page, Some(78)).into_iter().collect();
    let input = submit(&page, &client, &mut key, vec!["send-keys -t %1 -l z".into()]);
    commands.try_recv().unwrap().1.send(Ok(vec!["old screen".into()])).unwrap();
    screen.await;
    captured(&page, began);
    assert_eq!(frame(&page, true), None, "an older capture belongs to no new key");
    commands.try_recv().unwrap().1.send(Ok(vec![])).unwrap();
    input.await;
    let (began, screen) = capture(&page, &client, vec!["capture-pane -p -t %1".into()]);
    commands.try_recv().unwrap().1.send(Ok(vec!["z".into()])).unwrap();
    screen.await;
    captured(&page, began);
    assert_eq!(frame(&page, true).unwrap()["tr"]["ks"][0]["k"], 78);
}

#[tokio::test]
async fn refused_input_keeps_its_tmux_error() {
    let (page, _rx) = page();
    let (client, mut commands) = Control::test_queue();
    let mut key = arrived(&page, Some(79)).into_iter().collect();
    let input = submit(&page, &client, &mut key, vec!["send-keys -t %999 -l z".into()]);
    commands.try_recv().unwrap().1.send(Err("can't find pane".into())).unwrap();
    assert_eq!(input.await, vec![Err("can't find pane".into())]);
}
