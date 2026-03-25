use moonlitt_runtime::transport::{Transport, TransportState};

#[test]
fn transport_initial_state_is_stopped() {
    let t = Transport::new();
    assert_eq!(t.state(), TransportState::Stopped);
    assert!(!t.is_playing());
}

#[test]
fn transport_play_pause_stop() {
    let t = Transport::new();
    t.play();
    assert_eq!(t.state(), TransportState::Playing);
    assert!(t.is_playing());

    t.pause();
    assert_eq!(t.state(), TransportState::Paused);
    assert!(!t.is_playing());

    t.play();
    assert!(t.is_playing());

    t.stop();
    assert_eq!(t.state(), TransportState::Stopped);
}

#[test]
fn transport_tempo() {
    let t = Transport::new();
    assert!((t.tempo() - 120.0).abs() < 0.001); // default 120 BPM
    t.set_tempo(140.0);
    assert!((t.tempo() - 140.0).abs() < 0.001);
}

#[test]
fn transport_loop() {
    let t = Transport::new();
    assert!(!t.looping());
    t.set_loop(true);
    assert!(t.looping());
}

#[test]
fn transport_is_thread_safe() {
    use std::sync::Arc;
    use std::thread;

    let t = Arc::new(Transport::new());
    let t2 = t.clone();

    let writer = thread::spawn(move || {
        for _ in 0..1000 {
            t2.play();
            t2.set_tempo(130.0);
            t2.pause();
            t2.stop();
        }
    });

    for _ in 0..1000 {
        let _ = t.state();
        let _ = t.tempo();
        let _ = t.is_playing();
    }

    writer.join().unwrap();
}
