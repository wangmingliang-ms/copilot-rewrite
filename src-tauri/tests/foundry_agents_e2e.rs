use copilot_rewrite_lib::foundry::FoundryClient;
use std::process::Command;
use std::sync::OnceLock;

const DEFAULT_PROJECT_ENDPOINT: &str =
    "https://wangmi-ai.services.ai.azure.com/api/projects/wangmi-ai-project";

static ACCESS_TOKEN: OnceLock<String> = OnceLock::new();

fn azure_access_token() -> &'static str {
    ACCESS_TOKEN.get_or_init(|| {
        let az = if cfg!(windows) { "az.cmd" } else { "az" };
        let output = Command::new(az)
            .args([
                "account",
                "get-access-token",
                "--resource",
                "https://ai.azure.com",
                "--query",
                "accessToken",
                "--output",
                "tsv",
            ])
            .output()
            .expect("Azure CLI is required for Foundry E2E tests");

        assert!(
            output.status.success(),
            "Failed to get Azure access token: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8(output.stdout)
            .expect("Azure CLI token output was not UTF-8")
            .trim()
            .to_string()
    })
}

async fn invoke(agent_name: &str, input: &str) -> String {
    invoke_with_callback(agent_name, input, None).await
}

async fn invoke_with_callback(
    agent_name: &str,
    input: &str,
    on_chunk: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> String {
    let project_endpoint = std::env::var("FOUNDRY_PROJECT_ENDPOINT")
        .unwrap_or_else(|_| DEFAULT_PROJECT_ENDPOINT.to_string());

    FoundryClient::new()
        .process(
            &project_endpoint,
            agent_name,
            input,
            Some(azure_access_token()),
            on_chunk,
            None,
        )
        .await
        .unwrap_or_else(|error| panic!("{agent_name} invocation failed: {error:#}"))
}

fn contains_chinese(text: &str) -> bool {
    text.chars().any(|character| {
        ('\u{4e00}'..='\u{9fff}').contains(&character)
            || ('\u{3400}'..='\u{4dbf}').contains(&character)
    })
}

#[tokio::test]
#[ignore = "calls the live Microsoft Foundry project"]
async fn translate_agent_translates_chinese_to_english() {
    let chunks = std::sync::Mutex::new(Vec::<String>::new());
    let on_chunk = |text: &str| chunks.lock().unwrap().push(text.to_string());
    let output = invoke_with_callback(
        "copilot-rewrite-translate",
        "这是一个端到端测试。",
        Some(&on_chunk),
    )
    .await;

    assert!(!output.trim().is_empty());
    assert!(!contains_chinese(&output), "Expected English: {output}");
    let chunks = chunks.into_inner().unwrap();
    assert!(!chunks.is_empty(), "Expected streamed output chunks");
    assert_eq!(chunks.last(), Some(&output));
}

#[tokio::test]
#[ignore = "calls the live Microsoft Foundry project"]
async fn polish_agent_keeps_chinese_and_improves_the_text() {
    let input = "这个功能我觉得挺好的但是可能还可以改进一下。";
    let output = invoke("copilot-rewrite-polish", input).await;

    assert!(contains_chinese(&output));
    assert_ne!(output.trim(), input);
}

#[tokio::test]
#[ignore = "calls the live Microsoft Foundry project"]
async fn translate_polish_agent_returns_bilingual_sections() {
    let output = invoke(
        "copilot-rewrite-translate-polish",
        "我们明天完成这个功能，然后给团队演示。",
    )
    .await;

    assert!(contains_chinese(&output));
    assert!(output.contains("---TRANSLATED---"), "{output}");
}

#[tokio::test]
#[ignore = "calls the live Microsoft Foundry project"]
async fn creative_translate_agent_returns_english() {
    let output = invoke(
        "copilot-rewrite-creative-translate",
        "这个想法很好，我们应该继续推进。",
    )
    .await;

    assert!(!output.trim().is_empty());
    assert!(!contains_chinese(&output), "Expected English: {output}");
}

#[tokio::test]
#[ignore = "calls the live Microsoft Foundry project"]
async fn creative_polish_agent_keeps_chinese() {
    let output = invoke(
        "copilot-rewrite-creative-polish",
        "这个项目很重要需要大家一起努力把它做好。",
    )
    .await;

    assert!(contains_chinese(&output));
}

#[tokio::test]
#[ignore = "calls the live Microsoft Foundry project"]
async fn creative_translate_polish_agent_returns_bilingual_sections() {
    let output = invoke(
        "copilot-rewrite-creative-translate-polish",
        "我们希望这个工具能够帮助大家更自然地沟通。",
    )
    .await;

    assert!(contains_chinese(&output));
    assert!(output.contains("---TRANSLATED---"), "{output}");
}

#[tokio::test]
#[ignore = "calls the live Microsoft Foundry project"]
async fn read_agent_translates_english_to_chinese() {
    let output = invoke(
        "copilot-rewrite-read",
        "Microsoft Foundry provides a unified platform for building and evaluating AI applications.",
    )
    .await;

    assert!(contains_chinese(&output), "Expected Chinese: {output}");
}
