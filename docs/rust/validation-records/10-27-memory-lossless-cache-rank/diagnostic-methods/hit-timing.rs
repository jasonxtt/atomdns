use mosdns_native_host::{CacheTestClock,NativeCacheAdapter};
use std::{hint::black_box,rc::Rc,time::Instant};
unsafe extern "C" { fn clock()->i64; }
fn packet(index: usize, large: bool) -> (Vec<u8>, Vec<u8>) {
    let name = format!("item-{index}.fixture.test");
    let kind = if large { 16_u16 } else { 1 };
    let mut query = vec![0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        query.push(label.len().try_into().unwrap());
        query.extend_from_slice(label.as_bytes());
    }
    query.push(0);
    query.extend_from_slice(&kind.to_be_bytes());
    query.extend_from_slice(&1_u16.to_be_bytes());
    let mut wire = query.clone();
    wire[2] = 0x81;
    wire[7] = 1;
    wire.extend_from_slice(&[0xc0, 12]);
    wire.extend_from_slice(&kind.to_be_bytes());
    wire.extend_from_slice(&1_u16.to_be_bytes());
    wire.extend_from_slice(&300_u32.to_be_bytes());
    let data = if large {
        let mut data = Vec::new();
        for _ in 0..16 {
            data.push(255);
            data.extend_from_slice(&[b'x'; 255]);
        }
        data
    } else {
        vec![192, 0, 2, 1]
    };
    wire.extend_from_slice(&u16::try_from(data.len()).unwrap().to_be_bytes());
    wire.extend_from_slice(&data);
    (query, wire)
}


fn main(){
 for large in [false,true] {
  let clock=CacheTestClock::new(0);
  let adapter=NativeCacheAdapter::with_options_and_clock(10000,600,Rc::new(clock.clone())).unwrap();
  let (mut query,wire)=packet(0,large);
  assert!(adapter.begin_store(&query).unwrap().unwrap().publish_with_domain(&wire,"route-A").unwrap());
  for (stage,now) in [("fresh",0),("lazy",301)] {
   clock.set(now);
   for _ in 0..10000 {black_box(adapter.lookup(&query).unwrap().unwrap());}
   let cpu=unsafe{clock_process()};let began=Instant::now();let mut checksum=0u64;
   for i in 0..2000000u32 {
    query[..2].copy_from_slice(&(i as u16).to_be_bytes());
    let answer=adapter.lookup(black_box(&query)).unwrap().unwrap();
    checksum=checksum.wrapping_add(u64::from(answer[0])+u64::from(answer[answer.len()-1]));
    black_box(answer);
   }
   let wall=began.elapsed().as_secs_f64();let cpu=(unsafe{clock_process()}-cpu) as f64/1000000.0;
   println!("{{\"stage\":\"{stage}\",\"large\":{large},\"count\":2000000,\"wall_s\":{wall},\"cpu_s\":{cpu},\"checksum\":{checksum}}}");
  }
 }
}
unsafe fn clock_process()->i64 {unsafe {clock()}}
