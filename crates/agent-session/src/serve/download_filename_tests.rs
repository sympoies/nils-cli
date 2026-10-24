use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn image_download_filename_keeps_the_original_upload_name_and_response() {
    let fixture = Fixture::new();
    let image = fixture.upload("own", "photo%201.png", &fixture.png).await;
    fixture.append(json!([{"type":"input_text","text":format!("\"{}\"", image.display())}]));
    let page = fixture.refresh().await;
    let id = page["data"]["messages"][0]["images"][0]["id"]
        .as_str()
        .unwrap();
    let response = fixture.image(id).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-disposition")
            .and_then(|h| h.to_str().ok()),
        Some("attachment; filename*=UTF-8''photo%201.png")
    );
    assert_eq!(response.headers()[CONTENT_TYPE], "image/png");
    assert_eq!(
        response.headers()["content-length"],
        fixture.png.len().to_string()
    );
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap()
            .as_ref(),
        fixture.png
    );
}

#[tokio::test]
async fn image_download_filename_strips_path_components_and_control_characters() {
    let fixture = Fixture::new();
    let image = fixture
        .upload("own", "..%2Ffolder%5Cphoto%201%0D%0A.png", &fixture.png)
        .await;
    fixture.append(json!([{"type":"input_text","text":format!("\"{}\"", image.display())}]));
    let page = fixture.refresh().await;
    let id = page["data"]["messages"][0]["images"][0]["id"]
        .as_str()
        .unwrap();
    let names = image
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("attachment-names");
    fs::create_dir_all(&names).unwrap();
    fs::write(
        names.join(format!(
            "{}.json",
            image.file_name().unwrap().to_str().unwrap()
        )),
        serde_json::to_vec("../../folder\\photo 1\r\n.png").unwrap(),
    )
    .unwrap();
    assert_eq!(
        fixture
            .image(id)
            .await
            .headers()
            .get("content-disposition")
            .and_then(|h| h.to_str().ok()),
        Some("attachment; filename*=UTF-8''photo%201.png")
    );
}

#[tokio::test]
async fn image_download_filename_is_omitted_when_the_source_name_is_unknown() {
    let fixture = Fixture::new();
    fixture.append(
        json!([{"type":"input_image","image_url":format!("data:image/png;base64,{PNG}")} ]),
    );
    let page = fixture.refresh().await;
    let id = page["data"]["messages"][0]["images"][0]["id"]
        .as_str()
        .unwrap();
    let response = fixture.image(id).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("content-disposition").is_none());
    assert_eq!(response.headers()[CONTENT_TYPE], "image/png");
    assert_eq!(
        response.headers()["content-length"],
        fixture.png.len().to_string()
    );
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap()
            .as_ref(),
        fixture.png
    );
}
