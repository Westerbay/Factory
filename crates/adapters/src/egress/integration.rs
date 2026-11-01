use super::*;
use bollard::{
    API_DEFAULT_VERSION, Docker,
    exec::{CreateExecOptions, StartExecResults},
    query_parameters::*,
};
use chrono::Utc;
use factory_core::environments::*;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{net::Ipv4Addr, sync::Arc, time::Duration};
use uuid::Uuid;

fn environment(installation: Uuid) -> Environment {
    let id = Uuid::new_v4();
    Environment {
        id,
        user_id: Uuid::new_v4(),
        installation_id: installation,
        desired_state: DesiredState::Running,
        observed_state: ObservedState::Running,
        generation: 1,
        provisioning_state: ProvisioningState::VolumeReady,
        volume_name: format!("unused-egress-{id}"),
        container_name: format!("factory-egress-fixture-{id}"),
        image_id: String::new(),
        policy_version: "3".into(),
        observed_at: None,
        error_code: None,
        next_reconcile_at: Utc::now(),
    }
}

async fn command(docker: &Docker, container: &str, script: &str) -> (i64, String) {
    command_as(docker, container, script, None).await
}

async fn command_as(
    docker: &Docker,
    container: &str,
    script: &str,
    user: Option<&str>,
) -> (i64, String) {
    let exec = docker
        .create_exec(
            container,
            CreateExecOptions {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(vec!["/bin/bash", "-c", script]),
                user,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut text = String::new();
    if let StartExecResults::Attached { mut output, .. } =
        docker.start_exec(&exec.id, None).await.unwrap()
    {
        while let Some(frame) = tokio::time::timeout(Duration::from_secs(10), output.next())
            .await
            .unwrap()
        {
            text.push_str(&frame.unwrap().to_string());
            assert!(text.len() < 32768, "bounded fixture output");
        }
    }
    let status = docker
        .inspect_exec(&exec.id)
        .await
        .unwrap()
        .exit_code
        .unwrap();
    (status, text)
}

async fn client(docker: &Docker, env: &Environment, network: &UserNetwork, image: &str) -> String {
    let result = docker.create_container(Some(CreateContainerOptions { name: Some(env.container_name.clone()), ..Default::default() }),
        serde_json::from_value(json!({
            "Image":image, "User":"10001:10001", "Entrypoint":["/bin/sleep"], "Cmd":["infinity"],
            "Labels": {"io.factory.installation":env.installation_id.to_string(),"io.factory.environment":env.id.to_string(),"io.factory.user":env.user_id.to_string()},
            "HostConfig":{"NetworkMode":network.name,"ReadonlyRootfs":true,"CapDrop":["ALL"],"SecurityOpt":["no-new-privileges"]}
        })).unwrap()).await.unwrap();
    docker
        .start_container(&result.id, None::<StartContainerOptions>)
        .await
        .unwrap();
    result.id
}

#[tokio::test]
#[ignore = "explicit local Docker fixture: FACTORY_EGRESS_TEST_IMAGE_ID required"]
async fn real_egress_isolation_and_capability_drop() {
    let image = std::env::var("FACTORY_EGRESS_TEST_IMAGE_ID")
        .expect("explicit immutable proxy fixture image required");
    let installation = Uuid::new_v4();
    let metadata = EgressMetadata {
        schema_version: 1,
        installation_id: installation,
        image_id: image.clone(),
        host_ipv4: vec![Ipv4Addr::LOCALHOST],
    };
    metadata.validate().unwrap();
    #[cfg(windows)]
    let docker =
        Docker::connect_with_named_pipe("//./pipe/docker_engine", 30, API_DEFAULT_VERSION).unwrap();
    #[cfg(unix)]
    let docker =
        Docker::connect_with_unix("/var/run/docker.sock", 30, API_DEFAULT_VERSION).unwrap();
    let runtime = DockerEgress::new(docker.clone(), metadata.clone(), Arc::new(|| true)).unwrap();
    let a = environment(installation);
    let b = environment(installation);
    let mut witness = b.clone();
    witness.container_name = format!("{}-witness", b.container_name);
    let listener_image = std::env::var("FACTORY_EGRESS_DNS_IMAGE_ID")
        .expect("explicit fixture image with Python required");
    // Catch assertion failures so fixture-owned resources are still inspected and removed.
    use futures_util::FutureExt;
    let outcome = std::panic::AssertUnwindSafe(async {
        let network_a = runtime.prepare_user_network(&a).await.unwrap();
        let network_b = runtime.prepare_user_network(&b).await.unwrap();
        let client_a = client(&docker, &a, &network_a, &image).await;
        let client_b = client(&docker, &b, &network_b, &listener_image).await;
        runtime.inspect_user_network(&a).await.unwrap();
        runtime.inspect_user_network(&b).await.unwrap();
        // A separate client on B's bridge establishes that the target really accepts
        // connections. A closed port would not be evidence of inter-user isolation.
        let witness_b = client(&docker, &witness, &network_b, &image).await;
        let listener = docker.create_exec(&client_b, CreateExecOptions {
            cmd: Some(vec!["python3", "-c", "import socket; s=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); s.bind(('0.0.0.0',3128)); s.listen();\nwhile True:\n c,a=s.accept(); c.sendall(b'factory-owned-listener\\n'); c.close()"]),
            ..Default::default()
        }).await.unwrap();
        docker.start_exec(&listener.id, Some(bollard::exec::StartExecOptions { detach:true, ..Default::default() })).await.unwrap();
        let mut ready = false;
        for _ in 0..30 {
            let (code, _) = command(&docker, &client_a, "timeout 1 bash -c 'exec 3<>/dev/tcp/factory-egress/3128'").await;
            if code == 0 { ready = true; break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(ready, "proxy did not become ready");
        let (code, status) = command(&docker, &metadata.proxy_name(), "cat /proc/1/status; cat /proc/sys/net/ipv4/ip_forward; cat /proc/sys/net/ipv6/conf/all/forwarding").await;
        assert_eq!(code, 0);
        assert!(status.contains("Uid:\t10001\t10001\t10001\t10001"));
        assert!(status.contains("CapEff:\t0000000000000000"));
        assert!(status.contains("CapBnd:\t0000000000000000"));
        assert!(status.contains("NoNewPrivs:\t1"));
        assert!(status.ends_with("0\n0\n"));
        for authority in ["127.0.0.1:443", "169.254.169.254:443", "10.0.0.1:443", "[::1]:443", "api.openai.com:80", "ssh.github.com:22", "github.com:443", "example.com:443"] {
            let script = format!("timeout 3 bash -c 'exec 3<>/dev/tcp/factory-egress/3128; printf \"CONNECT {authority} HTTP/1.1\\r\\nHost: {authority}\\r\\n\\r\\n\" >&3; while IFS= read -r -t 1 line <&3; do printf \"%s\\n\" \"$line\"; done; true'");
            let (code, response) = command(&docker, &client_a, &script).await;
            assert_eq!(code, 0, "{authority}: {response}");
            assert!(response.contains("403"), "{authority}: {response}");
        }
        let b_inspect: Value = serde_json::to_value(docker.inspect_container(&client_b, None::<InspectContainerOptions>).await.unwrap()).unwrap();
        let b_ip = b_inspect["NetworkSettings"]["Networks"][&network_b.name]["IPAddress"].as_str().unwrap();
        let positive = format!("timeout 2 bash -c 'exec 3<>/dev/tcp/{b_ip}/3128; IFS= read -r -t 1 line <&3; printf \"%s\" \"$line\"'");
        let mut listener_ready = false;
        for _ in 0..20 {
            if command(&docker, &witness_b, &positive).await == (0, "factory-owned-listener".into()) { listener_ready = true; break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(listener_ready, "B listener must be reachable from B's authorized bridge client");
        assert_ne!(command(&docker, &client_a, &format!("timeout 2 bash -c 'exec 3<>/dev/tcp/{b_ip}/3128'")).await.0, 0);
        let via_proxy = format!("timeout 3 bash -c 'exec 3<>/dev/tcp/factory-egress/3128; printf \"CONNECT {b_ip}:3128 HTTP/1.1\\r\\nHost: {b_ip}:3128\\r\\n\\r\\n\" >&3; IFS= read -r -t 1 line <&3; printf \"%s\" \"$line\"'");
        assert!(command(&docker, &client_a, &via_proxy).await.1.contains("403"));
        assert_ne!(command(&docker, &client_a, "timeout 2 bash -c 'exec 3<>/dev/tcp/198.18.0.1/443'").await.0, 0);
        let (_, routes) = command(&docker, &client_a, "cat /proc/net/route").await;
        assert!(!routes.lines().skip(1).any(|line| line.split_whitespace().nth(1) == Some("00000000")), "internal client has a default route");
        docker.stop_container(&metadata.proxy_name(), Some(StopContainerOptions { t: Some(2), ..Default::default() })).await.unwrap();
        assert!(!runtime.available().await.unwrap());
        assert_eq!(command(&docker, &witness_b, &positive).await, (0, "factory-owned-listener".into()));
        assert_ne!(command(&docker, &client_a, &format!("timeout 2 bash -c 'exec 3<>/dev/tcp/{b_ip}/3128'")).await.0, 0);
        runtime.inspect_user_network(&a).await.unwrap();
        assert_eq!(command(&docker, &client_a, "printf local-shell-still-works").await, (0, "local-shell-still-works".into()));
    }).catch_unwind().await;
    // Exact deterministic fixture names and installation labels; never prune or remove volumes.
    for name in [
        &a.container_name,
        &b.container_name,
        &witness.container_name,
        &metadata.proxy_name(),
    ] {
        if let Ok(inspected) = docker
            .inspect_container(name, None::<InspectContainerOptions>)
            .await
        {
            let value = serde_json::to_value(inspected).unwrap();
            if outcome.is_err() {
                eprintln!("owned fixture: {value}");
            }
            assert_eq!(
                value["Config"]["Labels"]["io.factory.installation"],
                installation.to_string()
            );
            docker
                .remove_container(
                    value["Id"].as_str().unwrap(),
                    Some(RemoveContainerOptions {
                        force: true,
                        v: false,
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();
        }
    }
    for name in [
        metadata.user_network_name(a.id),
        metadata.user_network_name(b.id),
        metadata.outbound_name(),
    ] {
        if let Ok(inspected) = docker
            .inspect_network(&name, None::<InspectNetworkOptions>)
            .await
        {
            let value = serde_json::to_value(inspected).unwrap();
            if outcome.is_err() {
                eprintln!("owned fixture: {value}");
            }
            assert_eq!(
                value["Labels"]["io.factory.installation"],
                installation.to_string()
            );
            docker
                .remove_network(value["Id"].as_str().unwrap())
                .await
                .unwrap();
        }
    }
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
#[ignore = "explicit Docker DNS fixture: FACTORY_EGRESS_DNS_IMAGE_ID required"]
async fn real_egress_dns_rebinding_and_actual_destination() {
    use futures_util::FutureExt;
    let image =
        std::env::var("FACTORY_EGRESS_DNS_IMAGE_ID").expect("explicit fixture image required");
    let installation = Uuid::new_v4();
    let metadata = EgressMetadata {
        schema_version: 1,
        installation_id: installation,
        image_id: image.clone(),
        host_ipv4: vec![Ipv4Addr::LOCALHOST],
    };
    metadata.validate().unwrap();
    #[cfg(windows)]
    let docker =
        Docker::connect_with_named_pipe("//./pipe/docker_engine", 30, API_DEFAULT_VERSION).unwrap();
    #[cfg(unix)]
    let docker =
        Docker::connect_with_unix("/var/run/docker.sock", 30, API_DEFAULT_VERSION).unwrap();
    let fixture_name = format!("factory-dns-fixture-{installation}");
    let public_name = format!("factory-public-fixture-{installation}");
    let labels = json!({"io.factory.installation":installation.to_string()});
    let outcome = std::panic::AssertUnwindSafe(async {
        docker.create_network(metadata.network_body(None)).await.unwrap();
        // Simulated public address on an INTERNAL isolated bridge: no host bridge address,
        // host route or external public destination is contacted by this fixture.
        docker.create_network(serde_json::from_value(json!({
            "Name":public_name,"Driver":"bridge","Internal":true,"EnableIPv6":false,
            "Options":{"com.docker.network.bridge.gateway_mode_ipv4":"isolated"},
            "IPAM":{"Config":[{"Subnet":"11.255.250.0/24"}]},"Labels":labels
        })).unwrap()).await.unwrap();
        let dns = docker.create_container(Some(CreateContainerOptions {name:Some(fixture_name.clone()),..Default::default()}),
            serde_json::from_value(json!({
                "Image":image,"Entrypoint":["/bin/sleep"],"Cmd":["infinity"],"Labels":labels,
                "HostConfig":{"NetworkMode":metadata.outbound_name(),"ReadonlyRootfs":true,"CapDrop":["ALL"],"Tmpfs":{"/tmp":"rw,noexec,nosuid,nodev,size=16777216"}}
            })).unwrap()).await.unwrap();
        docker.start_container(&dns.id,None::<StartContainerOptions>).await.unwrap();
        let value = serde_json::to_value(docker.inspect_container(&dns.id,None::<InspectContainerOptions>).await.unwrap()).unwrap();
        let private_ip = value["NetworkSettings"]["Networks"][metadata.outbound_name()]["IPAddress"].as_str().unwrap().to_owned();
        docker.connect_network(&public_name,serde_json::from_value(json!({"Container":dns.id,"EndpointConfig":{"IPAMConfig":{"IPv4Address":"11.255.250.2"}}})).unwrap()).await.unwrap();
        let exec = docker.create_exec(&dns.id,CreateExecOptions {
            cmd:Some(vec!["/usr/bin/python3".to_owned(),"/opt/factory-fixture.py".to_owned()]),
            env:Some(vec![format!("FIXTURE_PRIVATE_IP={private_ip}")]),..Default::default()
        }).await.unwrap();
        docker.start_exec(&exec.id, Some(bollard::exec::StartExecOptions {detach:true,..Default::default()})).await.unwrap();
        let mut proxy_body = serde_json::to_value(metadata.proxy_body()).unwrap();
        // This controlled resolver is fixture-only. Production uses Docker's resolver defaults.
        proxy_body["HostConfig"]["Dns"] = json!([private_ip]);
        let proxy = docker.create_container(Some(CreateContainerOptions{name:Some(metadata.proxy_name()),..Default::default()}),serde_json::from_value(proxy_body).unwrap()).await.unwrap();
        docker.start_container(&proxy.id,None::<StartContainerOptions>).await.unwrap();
        docker.connect_network(&public_name,serde_json::from_value(json!({"Container":proxy.id,"EndpointConfig":{"IPAMConfig":{"IPv4Address":"11.255.250.3"}}})).unwrap()).await.unwrap();
        let proxy_inspect = serde_json::to_value(docker.inspect_container(&proxy.id,None::<InspectContainerOptions>).await.unwrap()).unwrap();
        let proxy_ip = proxy_inspect["NetworkSettings"]["Networks"][metadata.outbound_name()]["IPAddress"].as_str().unwrap();
        let mut ready = false;
        for _ in 0..30 {
            if command(&docker,&dns.id,&format!("timeout 1 bash -c 'exec 3<>/dev/tcp/{proxy_ip}/3128'")).await.0 == 0 {ready=true;break;}
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(ready,"fixture proxy not ready");
        let connect = format!("timeout 4 bash -c 'exec 3<>/dev/tcp/{proxy_ip}/3128; printf \"CONNECT auth.openai.com:443 HTTP/1.1\\r\\nHost: auth.openai.com:443\\r\\n\\r\\n\" >&3; while IFS= read -r -t 1 line <&3; do printf \"%s\\n\" \"$line\"; done; true'");
        let (code, response) = command(&docker,&dns.id,&connect).await;
        assert_eq!(code,0,"{response}");
        assert!(response.contains("200"),"DNS/allowed fixture tunnel failed: {response}");
        for host in ["api.anthropic.com", "claude.ai", "claude.com", "platform.claude.com", "sso.claude.com"] {
            let request = format!("timeout 4 bash -c 'exec 3<>/dev/tcp/{proxy_ip}/3128; printf \"CONNECT {host}:443 HTTP/1.1\\r\\nHost: {host}:443\\r\\n\\r\\n\" >&3; while IFS= read -r -t 1 line <&3; do printf \"%s\\n\" \"$line\"; done; true'");
            let (code, response) = command(&docker, &dns.id, &request).await;
            assert_eq!(code, 0, "{host}: {response}");
            assert!(response.contains("200"), "{host}: {response}");
        }
        let denied = format!("timeout 4 bash -c 'exec 3<>/dev/tcp/{proxy_ip}/3128; printf \"CONNECT unrelated.example:443 HTTP/1.1\\r\\nHost: unrelated.example:443\\r\\n\\r\\n\" >&3; while IFS= read -r -t 1 line <&3; do printf \"%s\\n\" \"$line\"; done; true'");
        let (code, response) = command(&docker, &dns.id, &denied).await;
        assert_eq!(code, 0, "{response}");
        assert!(response.contains("403"), "unexpected unrestricted egress: {response}");
        // The same isolated DNS fixture also proves the exact GitHub SSH host is allowed.
        let github_connect = connect.replace("auth.openai.com", "ssh.github.com");
        let (code, response) = command(&docker, &dns.id, &github_connect).await;
        assert_eq!(code, 0, "{response}");
        assert!(response.contains("200"), "GitHub SSH tunnel failed: {response}");
        let (code, rules) = command(&docker, &proxy.id, "/usr/sbin/nft -n list chain inet factory_egress output").await;
        assert_eq!(code, 0);
        assert!(rules.lines().any(|line| {
            let words = line.split_whitespace().collect::<Vec<_>>();
            line.contains("skuid 0") && line.contains("udp dport 53")
                && words.windows(2).any(|pair| pair[0] == "packets" && pair[1].parse::<u64>().is_ok_and(|count| count > 0))
        }), "root-owned Docker DNS forwarding was not observed: {rules}");
        let direct_dns = format!("python3 -c 'import socket; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); s.settimeout(1); s.sendto(bytes.fromhex(\"1234010000010000000000000461757468066f70656e616903636f6d0000010001\"),(\"{private_ip}\",53)); s.recvfrom(4096)'");
        assert_ne!(command_as(&docker, &proxy.id, &direct_dns, Some("10001:10001")).await.0, 0, "unprivileged proxy bypassed its DNS exception");
        for mode in ["mixed","private"] {
            assert_eq!(command(&docker,&dns.id,&format!("printf {mode} > /tmp/dns-mode")).await.0,0);
            tokio::time::sleep(Duration::from_secs(2)).await;
            let (code,response) = command(&docker,&dns.id,&connect).await;
            assert_eq!(code,0,"{response}");
            assert!(response.contains("403"),"{mode}: {response}");
        }
        // Even bypassing Squid's DNS/ACL layer cannot connect to the real private target.
        assert_ne!(command_as(&docker,&proxy.id,&format!("timeout 2 bash -c 'exec 3<>/dev/tcp/{private_ip}/443'"),Some("10001:10001")).await.0,0);
        let (code,connections) = command(&docker,&dns.id,"cat /tmp/connections").await;
        assert_eq!(code,0);
        assert!(!connections.is_empty());
        assert!(connections.lines().all(|address| address == "11.255.250.2"),"private target contacted: {connections}");
    }).catch_unwind().await;
    for name in [metadata.proxy_name(), fixture_name] {
        if let Ok(inspected) = docker
            .inspect_container(&name, None::<InspectContainerOptions>)
            .await
        {
            let value = serde_json::to_value(inspected).unwrap();
            assert_eq!(
                value["Config"]["Labels"]["io.factory.installation"],
                installation.to_string()
            );
            docker
                .remove_container(
                    value["Id"].as_str().unwrap(),
                    Some(RemoveContainerOptions {
                        force: true,
                        v: false,
                        ..Default::default()
                    }),
                )
                .await
                .unwrap();
        }
    }
    for name in [public_name, metadata.outbound_name()] {
        if let Ok(inspected) = docker
            .inspect_network(&name, None::<InspectNetworkOptions>)
            .await
        {
            let value = serde_json::to_value(inspected).unwrap();
            assert_eq!(
                value["Labels"]["io.factory.installation"],
                installation.to_string()
            );
            docker
                .remove_network(value["Id"].as_str().unwrap())
                .await
                .unwrap();
        }
    }
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
