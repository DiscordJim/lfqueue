mod scq;

pub use scq::ScqQueue;
pub use scq::ScqError;
pub use scq::LcsqQueue;
#[cfg(fuzzing)]
pub use scq::ScqRing;

// #[cfg(test)]
#[cfg(fuzzing)]
pub use fuzzing::*;
// mod fuzzing;

// #[cfg(test)]
#[cfg(fuzzing)]
mod fuzzing {
    use std::{collections::VecDeque, sync::Mutex};

    use arbitrary::{Arbitrary, *};

    use crate::ScqError;


    #[derive(Arbitrary, Debug)]
    pub enum GrindInstr {
        Enqueue(usize),
        Dequeue
    }

    #[derive(Debug)]
    pub struct GrindConfiguration {
        pub order: usize,
        pub instructions: Vec<GrindInstr>
    }

    #[derive(Debug)]
    pub struct MockQueue<T> {
        queue: Mutex<VecDeque<T>>,
        size: usize
    }

    impl<T> MockQueue<T> {
        pub fn new(order: usize) -> Self {
            Self {
                queue: VecDeque::with_capacity(1 << order).into(),
                size: 1 << order
            }
        }
        pub fn enqueue(&self, value: T) -> Result<(), ScqError> {
            let mut handle = self.queue.lock().unwrap();
            if handle.len() >= self.size {
                return Err(ScqError::QueueFull);
            }
            handle.push_back(value);
            Ok(())
        }
        pub fn dequeue(&self) -> Option<T> {
            self.queue.lock().unwrap().pop_front()
        }
    }

    pub fn configure_grind(unstructed: &mut Unstructured) -> GrindConfiguration {
        let value = usize::arbitrary(unstructed).unwrap();

        let delta = value as f64 / usize::MAX as f64;
        let order = (14.0 * delta) as usize;

        let instructions = Vec::<GrindInstr>::arbitrary(unstructed).unwrap();
        // println!("Instructions: {:?}", instructions);

        // println!("Value: {:?}", order);

        GrindConfiguration {
            order,
            instructions
        }

    }

  

}

