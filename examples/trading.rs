use std::{sync::Arc, time::{Duration, Instant}};

use lfqueue::UnboundedQueue;



struct Order {
    value_usd: usize,
    time: Instant
}

/// Creates orders.
fn order_tracker(bounded: Arc<UnboundedQueue<Order>>) {

    loop {

        bounded.enqueue(Order {
            value_usd: rand::random_range(10_000..100_000),
            time: Instant::now()
        });
        std::thread::sleep(Duration::from_millis(100));
    }


}

/// Sends orders to the network.
fn network_poller(unbounded: Arc<UnboundedQueue<Order>>) {
    loop {
        // not the most efficient since this is just spinning
        if let Some(value) = unbounded.dequeue() {
            println!("Processed: USD${} in {}microsecs", value.value_usd, value.time.elapsed().as_millis());
        }
    }
}


pub fn main() {
    println!("Starting trading example.");

    let unbounded = Arc::new(UnboundedQueue::<Order>::new());

    std::thread::spawn({
        let unbounded = unbounded.clone();
        move || {
            order_tracker(unbounded);

        }
    });

    // Start poller.
    network_poller(unbounded);

}