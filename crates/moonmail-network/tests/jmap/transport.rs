use super::*;

#[test]
fn session_discovery_falls_back_and_checks_each_redirect_before_authentication() -> Result {
    for invalid in [
        "not a URL",
        "http://example.test/session",
        "http://name@127.0.0.1/session",
    ] {
        assert!(matches!(
            Client::discover(invalid, "fixture".into()),
            Err(JmapError::Protocol)
        ));
    }
    let (url, worker) = server(3, |n, base, r| match n {
        0 => {
            assert_eq!(r.path, "/session");
            Reply(404, vec![])
        },
        1 => {
            assert_eq!(r.path, "/.well-known/jmap");
            Reply(307, b"/discovered".into())
        },
        _ => {
            assert_eq!(r.path, "/discovered");
            session(base)
        },
    });
    assert_eq!(
        Client::discover(&url, "fixture".into())?.account_id(),
        "account"
    );
    worker.join().unwrap();
    let (url, worker) = server(1, |_, _, _| {
        Reply(302, b"https://other.example.test/session".into())
    });
    assert!(matches!(
        Client::discover(&url, "fixture".into()),
        Err(JmapError::Protocol)
    ));
    worker.join().unwrap();
    Ok(())
}

#[test]
fn mismatched_identity_and_malformed_provider_acknowledgements_fail() -> Result {
    let (url, worker) = server(6, |n, base, r| match n {
        0 => session(base),
        1 => method(&r, "Identity/get", json!({"list":[]})),
        2 => method(
            &r,
            "Identity/get",
            json!({"list":[{"id":"one","email":ACCOUNT},{"id":"two","email":ACCOUNT}]}),
        ),
        3 => json(json!({"methodResponses":[["Email/set",{"accountId":"another"},"a"]]})),
        4 => json(json!({"methodResponses":[["error",{"type":"serverPartialFail"},"a"]]})),
        _ => method(&r, "Email/set", json!({})),
    });
    let client = Client::discover(&url, "fixture".into())?;
    for _ in 0..2 {
        assert!(matches!(
            client.check_identity(ACCOUNT),
            Err(JmapError::Rejected)
        ));
    }
    assert!(matches!(
        client.destroy_email("email"),
        Err(JmapError::Protocol)
    ));
    assert!(matches!(
        client.destroy_email("email"),
        Err(JmapError::ServerFailure)
    ));
    assert!(matches!(
        client.destroy_email("email"),
        Err(JmapError::Protocol)
    ));
    worker.join().unwrap();
    Ok(())
}

#[test]
fn definite_submission_rejections_are_safe_to_surface_as_unsent() -> Result {
    for stage in ["upload", "import", "unauthorized", "rejected"] {
        let count = match stage {
            "upload" => 4,
            "import" => 5,
            _ => 6,
        };
        let (url, worker) = server(count, move |n, base, r| match n {
            0 => session(base),
            1 => method(
                &r,
                "Identity/get",
                json!({"list":[{"email":ACCOUNT,"id":"a"}]}),
            ),
            2 => method(&r, "Mailbox/get", folders()),
            3 => json(
                json!({"accountId":"account","size":if stage=="upload" {1}else{MIME.len()},"blobId":"b"}),
            ),
            4 if stage == "import" => method(
                &r,
                "Email/import",
                json!({"notCreated":{"message":{"type":"invalidEmail"}}}),
            ),
            4 => method(
                &r,
                "Email/import",
                json!({"created":{"message":{"id":"m"}}}),
            ),
            _ if stage == "unauthorized" => Reply(401, vec![]),
            _ => method(
                &r,
                "EmailSubmission/set",
                json!({"notCreated":{"send":{"type":"invalidRecipients"}}}),
            ),
        });
        let mut client = Client::discover(&url, "fixture".into())?;
        let e = client
            .submit(ACCOUNT, &["recipient@example.test".into()], MIME)
            .unwrap_err();
        assert_eq!(
            Client::failure(&e),
            SubmissionFailure::NotSubmitted,
            "{stage}"
        );
        worker.join().unwrap();
    }
    Ok(())
}

#[test]
fn expired_transport_and_invalid_discovery_templates_return_typed_errors() -> Result {
    let (url, worker) = server(1, |_, base, _| session(base));
    let client = Client::discover(&url, "fixture".into())?;
    worker.join().unwrap();
    assert!(matches!(client.events(), Err(JmapError::Http(_))));
    for template in ["http://[", "https://unrelated.example.test/api"] {
        let (url, worker) = server(1, move |_, base, _| {
            let mut data: Value = serde_json::from_slice(&session(base).1).unwrap();
            data["apiUrl"] = json!(template);
            json(data)
        });
        let client = Client::discover(&url, "fixture".into())?;
        assert!(matches!(
            client.destroy_email("m"),
            Err(JmapError::Protocol)
        ));
        worker.join().unwrap();
    }
    Ok(())
}

#[test]
fn transport_failures_before_submission_leave_the_outbox_retryable() -> Result {
    for stage in ["uploadHttp", "uploadJson", "importHttp"] {
        let (url, w) = server(
            if stage == "importHttp" { 5 } else { 4 },
            move |n, base, r| match n {
                0 => session(base),
                1 => method(
                    &r,
                    "Identity/get",
                    json!({"list":[{"email":ACCOUNT,"id":"a"}]}),
                ),
                2 => method(&r, "Mailbox/get", folders()),
                3 if stage == "importHttp" => {
                    json(json!({"accountId":"account","blobId":"b","size":MIME.len()}))
                },
                _ if stage == "uploadJson" => Reply(200, b"[".into()),
                _ => Reply(503, vec![]),
            },
        );
        let mut client = Client::discover(&url, "fixture".into())?;
        assert_eq!(
            Client::failure(
                &client
                    .submit(ACCOUNT, &["r@example.test".into()], MIME)
                    .unwrap_err()
            ),
            SubmissionFailure::NotSubmitted
        );
        w.join().unwrap();
    }
    Ok(())
}
