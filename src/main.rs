use anyhow::Error;
use rand::Rng;
use std::path::Path;

use flechasdb::db::build::{
    BuildEvent,
    Database,
    DatabaseBuilder,
    QueryEvent,
};
use flechasdb::db::build::proto::serialize_database;
use flechasdb::db::stored;
use flechasdb::db::stored::LoadDatabase;
use flechasdb::io::LocalFileSystem;
use flechasdb::linalg::{norm2, scale_in};
use flechasdb::vector::{BlockVectorSet, VectorSet};

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1) {
        None => generate(),
        Some(s) if s == "generate" => generate(),
        Some(s) if s == "load" => load(&args[2]),
        _ => {
            println!("usage: {} [generate|query]", args[0]);
            Ok(())
        },
    }
}

fn generate() -> Result<(), Error> {
    const N: usize = 5000; // number of vectors
    const M: usize = 1024; // vector size
    const D: usize = 8; // number of divisions
    const P: usize = 10; // number of partitions
    const C: usize = 25; // number of clusters for product quantization
    const K: usize = 10; // K-nearest neighbors
    const NP: usize = 3; // number of partitions to query
    // prepares the data
    let time = std::time::Instant::now();
    let mut data = vec![0.0f32; N * M];
    let mut rng = rand::thread_rng();
    rng.fill(&mut data[..]);
    let mut vs = BlockVectorSet::chunk(data, M.try_into()?)?;
    for i in 0..N {
        let v = vs.get_mut(i);
        scale_in(v, 1.0 / norm2(v));
    }
    println!("prepared data in {} μs", time.elapsed().as_micros());
    // builds a vector database
    let time = std::time::Instant::now();
    let mut event_time = std::time::Instant::now();
    let mut db = DatabaseBuilder::new(vs)
        .with_partitions(P.try_into().unwrap())
        .with_divisions(D.try_into().unwrap())
        .with_clusters(C.try_into().unwrap())
        .build_with_events(move |event| {
            match event {
                BuildEvent::StartingIdAssignment |
                BuildEvent::StartingPartitioning |
                BuildEvent::StartingSubvectorDivision |
                BuildEvent::StartingQuantization(_) => {
                    event_time = std::time::Instant::now();
                },
                BuildEvent::FinishedIdAssignment => {
                    println!(
                        "assigned vector IDs in {} μs",
                        event_time.elapsed().as_micros(),
                    );
                },
                BuildEvent::FinishedPartitioning => {
                    println!(
                        "partitioned data in {} μs",
                        event_time.elapsed().as_micros(),
                    );
                },
                BuildEvent::FinishedSubvectorDivision => {
                    println!(
                        "divided data in {} μs",
                        event_time.elapsed().as_micros(),
                    );
                },
                BuildEvent::FinishedQuantization(i) => {
                    println!(
                        "quantized division {} in {} μs",
                        i,
                        event_time.elapsed().as_micros(),
                    );
                },
            };
        })?;
    println!("built database in {} μs", time.elapsed().as_micros());
    // sets attributes
    for i in 0..N {
        if i % 2 == 0 {
            db.set_attribute_at(i, ("datum_id", format!("{}", i)))?;
        } // tests vector without attributes
    }
    // creates a random query vector
    let qv = random_query_vector(&mut rng, M);
    // queries k-NN
    let time = std::time::Instant::now();
    let mut event_time = std::time::Instant::now();
    let results = db.query_with_events(
        &qv,
        K.try_into().unwrap(),
        NP.try_into().unwrap(),
        move |event| {
            match event {
                QueryEvent::StartingPartitionSelection |
                QueryEvent::StartingPartitionQuery(_) |
                QueryEvent::StartingResultSelection => {
                    event_time = std::time::Instant::now();
                },
                QueryEvent::FinishedPartitionSelection => {
                    println!(
                        "selected partitions in {} μs",
                        event_time.elapsed().as_micros(),
                    );
                },
                QueryEvent::FinishedPartitionQuery(i) => {
                    println!(
                        "queried partition {} in {} μs",
                        i,
                        event_time.elapsed().as_micros(),
                    );
                },
                QueryEvent::FinishedResultSelection => {
                    println!(
                        "selected results in {} μs",
                        event_time.elapsed().as_micros(),
                    );
                },
            }
        },
    )?;
    println!("queried k-NN in {} μs", time.elapsed().as_micros());
    for (i, result) in results.iter().enumerate() {
        println!("{}: {:?}", i, result);
    }
    // saves the database
    let time = std::time::Instant::now();
    save_database(&db, "testdb")?;
    println!("saved database in {} μs", time.elapsed().as_micros());
    Ok(())
}

fn load<P>(path: P) -> Result<(), Error>
where
    P: AsRef<Path> + core::fmt::Debug,
{
    const K: usize = 10; // K-nearest neighbors
    const NP: usize = 3; // number of partitions to query
    // loads the database
    println!("loading database from {:?}", path);
    let time = std::time::Instant::now();
    let db = load_database(path)?;
    println!("loaded database in {} μs", time.elapsed().as_micros());
    // randomly generates a query vector.
    let mut rng = rand::thread_rng();
    let qv = random_query_vector(&mut rng, db.vector_size());
    for r in 0..2 {
        // queries k-NN
        let time = std::time::Instant::now();
        let mut event_time = std::time::Instant::now();
        let results = db.query_with_events(
            &qv,
            K.try_into().unwrap(),
            NP.try_into().unwrap(),
            move |event| {
                match event {
                    stored::QueryEvent::StartingQueryInitialization |
                    stored::QueryEvent::StartingPartitionSelection |
                    stored::QueryEvent::StartingPartitionQuery(_) |
                    stored::QueryEvent::StartingResultSelection => {
                        event_time = std::time::Instant::now();
                    },
                    stored::QueryEvent::FinishedQueryInitialization => {
                        println!(
                            "[{}] initialized query in {} μs",
                            r,
                            event_time.elapsed().as_micros(),
                        );
                    },
                    stored::QueryEvent::FinishedPartitionSelection => {
                        println!(
                            "[{}] selected partitions in {} μs",
                            r,
                            event_time.elapsed().as_micros(),
                        );
                    },
                    stored::QueryEvent::FinishedPartitionQuery(i) => {
                        println!(
                            "[{}] queried partition {} in {} μs",
                            r,
                            i,
                            event_time.elapsed().as_micros(),
                        );
                    },
                    stored::QueryEvent::FinishedResultSelection => {
                        println!(
                            "[{}] selected results in {} μs",
                            r,
                            event_time.elapsed().as_micros(),
                        );
                    },
                }
            },
        )?;
        println!("[{}] queried k-NN in {} μs", r, time.elapsed().as_micros());
        let time = std::time::Instant::now();
        for (i, result) in results.iter().enumerate() {
            let datum_id = db.get_attribute_of(&result, "datum_id")?;
            println!("{}: datum_id={:?}, {:?}", i, datum_id, result);
        }
        println!("[{}] listed results in {} μs", r, time.elapsed().as_micros());
    }
    Ok(())
}

fn save_database<VS, P>(
    db: &Database<f32, VS>,
    base_path: P,
) -> Result<(), Error>
where
    VS: VectorSet<f32>,
    P: AsRef<Path> + core::fmt::Debug,
{
    println!("saving database to {:?}", base_path);
    let mut fs = LocalFileSystem::new(base_path);
    serialize_database(db, &mut fs)?;
    Ok(())
}

fn load_database<P>(
    path: P,
) -> Result<stored::Database<f32, LocalFileSystem>, Error>
where
    P: AsRef<Path> + core::fmt::Debug,
{
    let path = path.as_ref();
    let db = stored::Database::<f32, _>::load_database(
        LocalFileSystem::new(path.parent().unwrap()),
        path.file_name().unwrap().to_str().unwrap(),
    )?;
    Ok(db)
}

fn random_query_vector<R>(rng: &mut R, size: usize) -> Vec<f32>
where
    R: Rng,
{
    let mut v: Vec<f32> = Vec::with_capacity(size);
    unsafe {
        let p = v.as_mut_ptr();
        rng.fill(core::slice::from_raw_parts_mut(p, size));
        v.set_len(size);
    }
    let norm = norm2(&v);
    scale_in(&mut v, 1.0 / norm);
    v
}
