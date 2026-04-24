use foundationdb::{
    options,
    tuple::{pack, pack_with_versionstamp, unpack, Subspace, Versionstamp},
    Database, FdbBindingError, RangeOption,
};
use futures::StreamExt;

#[tokio::main]
async fn main() {
    let network = unsafe { foundationdb::boot() };

    run_versionstamp_key_example()
        .await
        .expect("failed to run versionstamp example");

    run_versionstamp_value_example()
        .await
        .expect("failed to run versionstamp example");

    drop(network);
}

async fn run_versionstamp_key_example() -> Result<(), FdbBindingError> {
    println!("running example for setting versionstamped keys");
    // Using versionstamps in order to create a sequential path.
    let db = Database::default()?;
    db.set_option(options::DatabaseOption::TransactionTimeout(5000))?;
    db.set_option(options::DatabaseOption::TransactionRetryLimit(3))?;

    let subspace = Subspace::all().subspace(&"versionstamp_example");
    let (from, to) = subspace.range();
    db.run(|trx, _maybe_committed| {
        let from = from.clone();
        let to = to.clone();
        async move {
            trx.clear_range(&from, &to);
            Ok(())
        }
    })
    .await?;

    // We can create two interleaved transactions, each creating a versionstamped key.
    // The first to commit will be the one to get the lowest versionstamp value.

    // versionstamps allow user ordering too, which can be set as the user version when creating an
    // incomplete versionstamp. While these two keys will be committed in the same transaction,
    // the versionstamp ordering is guaranteed to give us the expected order.
    let key_1_1 = subspace.pack_with_versionstamp(&("prefix", &Versionstamp::incomplete(2)));
    let key_1_2 = subspace.pack_with_versionstamp(&("prefix", &Versionstamp::incomplete(1)));

    let value_1_1 = "value_1_1";
    let value_1_2 = "value_1_2";

    // We can create a second set of kvs in a new transaction
    let key_2_1 = subspace.pack_with_versionstamp(&("prefix", &Versionstamp::incomplete(1)));
    let key_2_2 = subspace.pack_with_versionstamp(&("prefix", &Versionstamp::incomplete(2)));

    let value_2_1 = "value_2_1";
    let value_2_2 = "value_2_2";

    // The order in which we commit will determine the final ordering.
    // Committing the second transaction first will give us the expected order.
    db.run(|trx, _maybe_committed| {
        let key_2_1 = key_2_1.clone();
        let key_2_2 = key_2_2.clone();
        async move {
            // Creating versionstamped keys is an atomic op.
            trx.atomic_op(
                &key_2_1,
                &pack(&value_2_1),
                options::MutationType::SetVersionstampedKey,
            );
            trx.atomic_op(
                &key_2_2,
                &pack(&value_2_2),
                options::MutationType::SetVersionstampedKey,
            );
            Ok(())
        }
    })
    .await?;

    db.run(|trx, _maybe_committed| {
        let key_1_1 = key_1_1.clone();
        let key_1_2 = key_1_2.clone();
        async move {
            // Creating versionstamped keys is an atomic op.
            trx.atomic_op(
                &key_1_1,
                &pack(&value_1_1),
                options::MutationType::SetVersionstampedKey,
            );
            trx.atomic_op(
                &key_1_2,
                &pack(&value_1_2),
                options::MutationType::SetVersionstampedKey,
            );
            Ok(())
        }
    })
    .await?;

    // Reading the keys back will give us the expected order.
    // value_2_1
    // value_2_2
    // value_1_2
    // value_1_1

    db.run(|trx, _maybe_committed| {
        let from = from.clone();
        let to = to.clone();
        let subspace = subspace.clone();
        async move {
            let range = RangeOption::from((from, to));
            let mut kvs = trx.get_ranges_keyvalues(range, false);
            while let Some(kv) = kvs.next().await {
                let kv = kv?;
                let (_, v) = subspace
                    .unpack::<(String, Versionstamp)>(kv.key())
                    .expect("failed to unpack key");
                let value = unpack::<String>(kv.value()).expect("failed to unpack value");
                println!(
                    "{:?} {}: {}",
                    v.transaction_version(),
                    v.user_version(),
                    value
                );
            }
            Ok(())
        }
    })
    .await?;
    Ok(())
}

async fn run_versionstamp_value_example() -> Result<(), FdbBindingError> {
    println!("running example for setting versionstamped values");
    // You can use versionstamps in values too, for example to point to a versionstamped key.
    let db = Database::default()?;
    db.set_option(options::DatabaseOption::TransactionTimeout(5000))?;
    db.set_option(options::DatabaseOption::TransactionRetryLimit(3))?;

    let subspace = Subspace::all().subspace(&"versionstamp_example");
    let (from, to) = subspace.range();
    db.run(|trx, _maybe_committed| {
        let from = from.clone();
        let to = to.clone();
        async move {
            trx.clear_range(&from, &to);
            Ok(())
        }
    })
    .await?;

    // In our transaction we will create a versionstamped key, and then reference it in another
    // known "index" key.
    let key_tuple = ("data", &Versionstamp::incomplete(0));
    let key = subspace.pack_with_versionstamp(&key_tuple);
    let value = "some value";

    let index_key = subspace.pack(&"index");

    db.run(|trx, _maybe_committed| {
        let key = key.clone();
        let index_key = index_key.clone();
        async move {
            trx.atomic_op(
                &key,
                &pack(&value),
                options::MutationType::SetVersionstampedKey,
            );

            trx.atomic_op(
                &index_key,
                &pack_with_versionstamp(&key_tuple),
                options::MutationType::SetVersionstampedValue,
            );
            Ok(())
        }
    })
    .await?;

    // Now we created our versionstamped key and a reference to it.
    // We can read the index key and get the versionstamped key back.
    db.run(|trx, _maybe_committed| {
        let index_key = index_key.clone();
        let subspace = subspace.clone();
        async move {
            let index_kv = trx
                .get(&index_key, false)
                .await?
                .expect("didn't find index");
            let versionstamped_key =
                unpack::<(String, Versionstamp)>(&index_kv).expect("failed to unpack value");

            // notice that we don't pack with versionstamp here because the versionstamp we received is complete.
            let versionstamped_value = trx
                .get(&subspace.pack(&versionstamped_key), false)
                .await?
                .expect("didn't find reference");
            let value = unpack::<String>(&versionstamped_value).expect("failed to unpack value");
            println!("got back value {value}");
            Ok(())
        }
    })
    .await?;
    Ok(())
}
