//! Many images at once, an image per thread. The codec keeps nothing between images, so
//! this is the only way more than one thread helps; a single image is coded on one.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::{decode, encode, Error, FrameInfo};

/// [`encode`] for each of `images`, all of the size and depth of `frame`, on up to
/// `threads` threads. The streams come back in the order of the images.
///
/// With `threads` at 1 everything runs on the calling thread. On an error the rest of
/// the batch is dropped and the error of the first image that has one is returned, as
/// coding them one after the other would.
pub fn encode_many<P>(frame: FrameInfo, images: &[P], threads: usize) -> Result<Vec<Vec<u8>>, Error>
where
    P: AsRef<[u8]> + Sync,
{
    many(images, threads, |pixels| encode(frame, pixels.as_ref()))
}

/// [`decode`] for each of `streams`, on up to `threads` threads. The images come back in
/// the order of the streams, each with its own size: they need not be alike.
///
/// With `threads` at 1 everything runs on the calling thread. On an error the rest of
/// the batch is dropped and the error of the first stream that has one is returned.
pub fn decode_many<S>(streams: &[S], threads: usize) -> Result<Vec<(FrameInfo, Vec<u8>)>, Error>
where
    S: AsRef<[u8]> + Sync,
{
    many(streams, threads, |stream| decode(stream.as_ref()))
}

/// `work` on every item, on up to `threads` threads, results in the order of the items.
fn many<T, R>(
    items: &[T],
    threads: usize,
    work: impl Fn(&T) -> Result<R, Error> + Sync,
) -> Result<Vec<R>, Error>
where
    T: Sync,
    R: Send,
{
    if threads == 0 {
        return Err(Error::InvalidParameter("threads go from 1"));
    }
    let threads = threads.min(items.len());
    if threads <= 1 {
        return items.iter().map(work).collect();
    }

    // Threads take the items one by one, in order, until none is left or one failed.
    // An item is taken before the ones after it, and a thread finishes the item it took:
    // so every item before a failed one is done, and the failure with the lowest index
    // is the one a single thread would have met.
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let worker = || {
        let mut done = Vec::new();
        while !failed.load(Ordering::Relaxed) {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(index) else { break };
            let result = work(item);
            if result.is_err() {
                failed.store(true, Ordering::Relaxed);
            }
            done.push((index, result));
        }
        done
    };
    let mut results: Vec<(usize, Result<R, Error>)> = std::thread::scope(|scope| {
        // the calling thread is one of them
        let others: Vec<_> = (1..threads).map(|_| scope.spawn(worker)).collect();
        let mut all = worker();
        for other in others {
            match other.join() {
                Ok(done) => all.extend(done),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        all
    });
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn results_keep_the_order_of_the_items() {
        let items: Vec<u32> = (0..1000).collect();
        for threads in [1, 2, 3, 7, 64, 5000] {
            let doubled = many(&items, threads, |&n| Ok(n * 2)).expect("no error");
            assert!(
                doubled.iter().copied().eq(items.iter().map(|n| n * 2)),
                "{threads} threads"
            );
        }
    }

    #[test]
    fn the_first_error_is_returned_and_the_rest_is_dropped() {
        let items: Vec<u32> = (0..10_000).collect();
        for threads in [1, 2, 8] {
            let started = AtomicUsize::new(0);
            let result = many(&items, threads, |&n| {
                started.fetch_add(1, Ordering::Relaxed);
                match n {
                    40 => Err(Error::Truncated),
                    41..=60 => Err(Error::TooLarge),
                    _ => Ok(n),
                }
            });
            assert_eq!(result, Err(Error::Truncated), "{threads} threads");
            assert!(
                started.into_inner() < 1000,
                "{threads} threads went on after the error"
            );
        }
    }

    #[test]
    fn no_thread_is_an_error_and_no_item_is_no_work() {
        let none: [u32; 0] = [];
        assert_eq!(
            many(&[1u32], 0, |&n| Ok(n)),
            Err(Error::InvalidParameter("threads go from 1"))
        );
        assert_eq!(
            many(&none, 0, |&n| Ok(n)),
            Err(Error::InvalidParameter("threads go from 1"))
        );
        assert_eq!(many(&none, 4, |&n| Ok(n)), Ok(vec![]));
    }

    #[test]
    fn one_thread_is_the_calling_thread() {
        let caller = std::thread::current().id();
        let ran_on = many(&[(); 5], 1, |_| Ok(std::thread::current().id())).expect("no error");
        assert!(ran_on.iter().all(|&id| id == caller));
        // and a batch of one starts none either
        let ran_on = many(&[()], 8, |_| Ok(std::thread::current().id())).expect("no error");
        assert_eq!(ran_on, [caller]);
    }

    #[test]
    fn a_panic_in_a_thread_is_the_callers_panic() {
        let items: Vec<u32> = (0..100).collect();
        let caught = std::panic::catch_unwind(|| {
            many(
                &items,
                4,
                |&n| if n == 50 { panic!("item 50") } else { Ok(n) },
            )
        });
        assert!(caught.is_err());
    }
}
