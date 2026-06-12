use moonlitt_audio_io::AudioEvent;

#[test]
fn event_is_copy() {
    let e = AudioEvent::NoteOn {
        channel: 0,
        note: 60,
        velocity: 100,
    };
    let e2 = e; // Copy
    let e3 = e; // Still valid — it's Copy
    assert!(matches!(e2, AudioEvent::NoteOn { note: 60, .. }));
    assert!(matches!(e3, AudioEvent::NoteOn { note: 60, .. }));
}

#[test]
fn event_size_is_small() {
    // AudioEvent should fit comfortably in a cache line
    assert!(std::mem::size_of::<AudioEvent>() <= 16);
}

#[test]
fn event_queue_roundtrip() {
    use rtrb::RingBuffer;
    let (mut producer, mut consumer) = RingBuffer::<AudioEvent>::new(64);

    producer
        .push(AudioEvent::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        })
        .unwrap();
    producer.push(AudioEvent::SetVolume(0.5)).unwrap();
    producer.push(AudioEvent::AllNotesOff).unwrap();

    let e1 = consumer.pop().unwrap();
    assert!(matches!(e1, AudioEvent::NoteOn { note: 60, .. }));
    let e2 = consumer.pop().unwrap();
    assert!(matches!(e2, AudioEvent::SetVolume(v) if (v - 0.5).abs() < 0.001));
    let e3 = consumer.pop().unwrap();
    assert!(matches!(e3, AudioEvent::AllNotesOff));
    assert!(consumer.pop().is_err()); // empty
}

#[test]
fn event_queue_stress() {
    use rtrb::RingBuffer;
    use std::thread;

    let (mut producer, mut consumer) = RingBuffer::<AudioEvent>::new(1024);
    let count = 10_000usize;

    let writer = thread::spawn(move || {
        for i in 0..count {
            let event = AudioEvent::NoteOn {
                channel: 0,
                note: (i % 128) as u8,
                velocity: 100,
            };
            // Retry if full
            while producer.push(event).is_err() {
                thread::yield_now();
            }
        }
    });

    let mut received = 0;
    while received < count {
        if let Ok(_event) = consumer.pop() {
            received += 1;
        } else {
            thread::yield_now();
        }
    }

    writer.join().unwrap();
    assert_eq!(received, count);
}
