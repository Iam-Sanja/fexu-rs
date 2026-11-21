use argon2::{Algorithm, Argon2, Params, Version};
use std::sync::{Arc, Barrier};
fn main() {
    let count = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "24".into())
        .parse::<usize>()
        .unwrap();
    let stack = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "2097152".into())
        .parse::<usize>()
        .unwrap();
    let start = Arc::new(Barrier::new(count));
    let mut jobs = Vec::new();
    for _ in 0..count {
        let start = start.clone();
        jobs.push(
            std::thread::Builder::new()
                .stack_size(stack)
                .spawn(move || {
                    let params = Params::new(19_456, 2, 1, None).unwrap();
                    let hasher = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
                    start.wait();
                    for _ in 0..2 {
                        let mut hash = [0u8; 32];
                        hasher
                            .hash_password_into(
                                b"only-a-disposable-test-password",
                                b"disposable-salt",
                                &mut hash,
                            )
                            .unwrap();
                        assert_ne!(hash, [0; 32]);
                    }
                })
                .unwrap(),
        );
    }
    for job in jobs {
        job.join().unwrap();
    }
    println!("argon2-only passed: {count} threads, {stack} byte stacks");
}
