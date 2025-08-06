use lfqueue::UnboundedQueue;



pub fn main() {

    let mut queue = UnboundedQueue::new();
    loop {
        println!("Start...");
        for i in 0..1_000_000 {
         queue.enqueue(4);
            
        }
        for i in 0..1_000_000 {
            queue.dequeue();
        }
        crossbeam_epoch::pin().flush();
    
    }
}