use shared_mutex::{SharedMutex, unlink_if_exists};
use std::{env, process::Command, thread, time::Duration};

type Sharedu64 = SharedMutex<u64>;

fn main() {
    let args: Vec<String> = env::args().collect();
    let _ = unlink_if_exists("test_counter");

    if args.len() > 1 && args[1] == "child" {
        child_process();
    } else {
        parent_process();
    }
}

fn parent_process() {
    println!("=== Parent Process ===");

    let shared = unsafe { Sharedu64::new_with_val("test_counter", 0) };
    println!("Parent: Created shared counter");

    let mut child = Command::new(env::current_exe().unwrap())
        .arg("child")
        .spawn()
        .expect("Failed to spawn child");

    println!("Parent: Spawned child process");

    for i in 1..=5 {
        let mut guard = shared.lock().unwrap();
        let old_val = *guard;
        *guard += i;
        println!("Parent: {} -> {} (added {})", old_val, *guard, i);
        drop(guard);
        thread::sleep(Duration::from_millis(300));
    }

    child.wait().expect("Child process failed");

    let guard = shared.lock().unwrap();
    println!("Parent: Final value: {}", *guard);
}

fn child_process() {
    println!("  Child: Starting");

    thread::sleep(Duration::from_millis(100));

    let shared = unsafe { Sharedu64::from_name("test_counter") };
    println!("  Child: Connected to shared counter");

    for i in 1..=5 {
        let mut guard = shared.lock().unwrap();
        let old_val = *guard;
        *guard += i * 10;
        println!("  Child: {} -> {} (added {})", old_val, *guard, i * 10);
        drop(guard);
        thread::sleep(Duration::from_millis(200));
    }

    println!("  Child: Finished");
}
