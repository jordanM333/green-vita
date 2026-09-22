//! Discrete-event scheduling comparison, NOT a Vita decoder emulator.
//! AU completion times copied from the user's RX36 Cloud tail (75.818-75.991s).
//! Later AUs were dropped by RX36's IDR gate; replay asks what intact admission
//! would cost. No packet payload, firmware behavior, or network repair is replayed.
use std::collections::VecDeque;
use super::policy;
const ARRIVALS: &[(u64,u32)] = &[
    (75818020,3025984993),
    (75818178,3025986523),
    (75821787,3025988053),
    (75822169,3025989493),
    (75824461,3025991023),
    (75862535,3025992553),
    (75866749,3025993993),
    (75869747,3025995523),
    (75869898,3025997053),
    (75869995,3025998493),
    (75872367,3026000113),
    (75900705,3026001643),
    (75905076,3026003083),
    (75905174,3026004523),
    (75908218,3026006053),
    (75908316,3026007583),
    (75908405,3026009023),
    (75946541,3026010553),
    (75950604,3026012083),
    (75950730,3026013523),
    (75950829,3026015053),
    (75953456,3026016583),
    (75953536,3026018023),
    (75953613,3026019553),
    (75988079,3026021083),
    (75988163,3026022613),
    (75991162,3026024053),
];
#[derive(Default, Debug)]
struct Result { inputs: usize, drops: usize, max_queue: usize, polls: usize, max_wait_us: u64 }
fn replay(legacy: bool, capacity: usize) -> Result {
    let mut result=Result::default();
    let mut queue=VecDeque::new();
    let mut pending=VecDeque::new();
    let(mut now,mut next,mut consecutive)=(ARRIVALS[0].0,0,0);
    let(mut drain,mut input_after_poll)=(false,false);
    for _ in 0..1000 {
        while next<ARRIVALS.len() && ARRIVALS[next].0<=now {
            if queue.len()==capacity { result.drops+=1; }
            else { queue.push_back(ARRIVALS[next]); }
            result.max_queue=result.max_queue.max(queue.len());next+=1;
        }
        if queue.is_empty() && !drain {
            if next==ARRIVALS.len() { return result; }
            now=ARRIVALS[next].0;continue;
        }
        let poll=!input_after_poll && drain && !pending.is_empty() && if legacy {
            !(consecutive<2 && queue.len()>=2)
        } else { policy::poll_before_input(queue.len(),pending.len()) };
        // Explicit synthetic service model using representative trace costs:
        // output-producing input 2.6ms; empty call 8.2ms; output poll 3ms;
        // readiness after 40ms. These are assumptions, not measured new results.
        if poll {
            result.polls+=1;consecutive=0;
            now+=if pending.front().is_some_and(|at|*at<=now) {3000}else{8200};
            let produced=pending.front().is_some_and(|at|*at<=now);
            if produced {pending.pop_front();}
            drain=produced;input_after_poll=true;
        } else if let Some((at,_rtp))=queue.pop_front() {
            result.inputs+=1;consecutive+=1;
            result.max_wait_us=result.max_wait_us.max(now-at);
            pending.push_back(now+40_000);
            now+=if pending.front().is_some_and(|at|*at<=now) {2600}else{8200};
            if pending.front().is_some_and(|at|*at<=now) {pending.pop_front();}
            drain=!pending.is_empty();input_after_poll=false;
        } else {
            input_after_poll=false;
            if pending.is_empty() {drain=false;}
        }
    }
    panic!("scheduler model failed to settle");
}
#[test]
fn trace_shaped_replay_distinguishes_input_service_from_just_a_larger_queue() {
    let old=replay(true,6);
    let larger_only=replay(true,policy::AU_QUEUE_CAPACITY);
    let candidate=replay(false,policy::AU_QUEUE_CAPACITY);
    assert!(old.drops>0);
    assert_eq!(candidate.drops,0);
    assert_eq!(candidate.inputs,ARRIVALS.len());
    assert!(candidate.polls<larger_only.polls);
    assert!(candidate.max_queue<larger_only.max_queue);
    assert!(candidate.max_wait_us<larger_only.max_wait_us);
    println!("Synthetic trace-paced model: RX36={old:?}; capacity-only={larger_only:?}; candidate={candidate:?}");
}
