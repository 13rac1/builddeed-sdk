//! The shared test data in ../fixtures: every BuildDeed SDK passes it.

use builddeed::{fake_rpc, Client, Error, Product, Status};
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = format!("{}/../fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
        .expect("fixture JSON")
}

#[test]
fn device_id_vectors() {
    let f = fixture("device-id.json");
    let cases = f["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for c in cases {
        let got = builddeed::device_id_from(
            c["contract"].as_str().unwrap(),
            c["machine_id"].as_str().unwrap(),
        );
        assert_eq!(got, c["device_id"].as_str().unwrap(), "{c}");
    }
}

#[test]
fn confirm_code_vectors() {
    let f = fixture("confirm-code.json");
    let cases = f["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for c in cases {
        let d = c["device_id"].as_str().unwrap();
        assert_eq!(
            builddeed::confirm_code_from(d),
            c["code"].as_str().unwrap(),
            "{c}"
        );
        let client = Client::with_device(Product::testnet("c", "k", "1"), d);
        assert_eq!(client.confirm_code().unwrap(), c["code"].as_str().unwrap());
    }
}

#[test]
fn url_vectors() {
    let f = fixture("urls.json");
    let p = &f["product"];
    let product = Product::testnet(
        p["contract"].as_str().unwrap(),
        p["key"].as_str().unwrap(),
        p["version"].as_str().unwrap(),
    )
    .with_site_url(p["site_url"].as_str().unwrap());
    let client = Client::with_device(product, f["device_id"].as_str().unwrap());
    assert_eq!(
        client.register_url().unwrap(),
        f["register_url"].as_str().unwrap()
    );
    assert_eq!(client.buy_url(), f["buy_url"].as_str().unwrap());
    assert_eq!(
        client.short_device_id().unwrap(),
        f["short_device_id"].as_str().unwrap()
    );
}

#[test]
fn device_license_replies() {
    let f = fixture("device-license.json");
    for c in f["cases"].as_array().unwrap() {
        let url = fake_rpc::serve(vec![(200, c["body"].to_string())]);
        let client = Client::with_device(
            Product::testnet("doom.builddeed.testnet", "doom", "0.2.0").with_rpc_url(url),
            "a".repeat(64),
        );
        let got = client.status();
        let name = c["name"].as_str().unwrap();
        match c["expect"].as_str().unwrap() {
            "seat" => match got {
                Ok(Status::Registered(s)) => {
                    assert_eq!(s.account_id, c["account_id"].as_str().unwrap(), "{name}");
                    assert_eq!(s.token_id, c["token_id"].as_str().unwrap(), "{name}");
                }
                other => panic!("{name}: {other:?}"),
            },
            "ended" => assert_eq!(
                got,
                Ok(Status::LicenseEnded {
                    token_id: c["token_id"].as_str().unwrap().into()
                }),
                "{name}"
            ),
            "none" => assert_eq!(got, Ok(Status::NoSeat), "{name}"),
            "rpc_error" => assert!(matches!(got, Err(Error::Rpc(_))), "{name}: {got:?}"),
            e => panic!("unknown expect {e}"),
        }
    }
}
