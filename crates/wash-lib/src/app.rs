//! Interact with and manage wadm applications over NATS, requires the `nats` feature

use std::{path::PathBuf, time::Duration};

use anyhow::{bail, Result};
use async_nats::{Client, Message};
use wadm::server::{
    DeleteModelRequest, DeleteModelResponse, DeployModelRequest, DeployModelResponse,
    GetModelRequest, GetModelResponse, ModelSummary, PutModelResponse, UndeployModelRequest,
    VersionResponse,
};

use tokio::io::AsyncReadExt;
use url::Url;

use crate::config::DEFAULT_LATTICE_PREFIX;

/// The NATS prefix wadm's API is listening on
const WADM_API_PREFIX: &str = "wadm.api";

/// A helper enum to easily refer to wadm model operations and then use the
/// [ToString](ToString) implementation for NATS topic formation
pub enum ModelOperation {
    List,
    Get,
    History,
    Delete,
    Put,
    Deploy,
    Undeploy,
}

impl ToString for ModelOperation {
    fn to_string(&self) -> String {
        match self {
            ModelOperation::List => "list",
            ModelOperation::Get => "get",
            ModelOperation::History => "versions",
            ModelOperation::Delete => "del",
            ModelOperation::Put => "put",
            ModelOperation::Deploy => "deploy",
            ModelOperation::Undeploy => "undeploy",
        }
        .to_string()
    }
}

pub enum AppManifest {
    SerializedModel(String),
    ModelName(String),
}

/// Undeploy a model, instructing wadm to no longer manage the given application
///
/// # Arguments
/// * `client` - The [Client](async_nats::Client) to use in order to send the request message
/// * `lattice_prefix` - Optional lattice prefix that the application is managed on, defaults to `default`
/// * `model_name` - Model name to undeploy
/// * `non_destructive` - Undeploy deletes managed resources by default, this can be overridden by setting this to `true`
pub async fn undeploy_model(
    client: &Client,
    lattice_prefix: Option<String>,
    model_name: &str,
    non_destructive: bool,
) -> Result<DeployModelResponse> {
    let res = model_request(
        client,
        ModelOperation::Undeploy,
        lattice_prefix,
        Some(model_name),
        serde_json::to_vec(&UndeployModelRequest { non_destructive })?,
    )
    .await?;

    serde_json::from_slice(&res.payload).map_err(|e| anyhow::anyhow!(e))
}

/// Deploy a model, instructing wadm to manage the application
///
/// # Arguments
/// * `client` - The [Client](async_nats::Client) to use in order to send the request message
/// * `lattice_prefix` - Optional lattice prefix that the application will be managed on, defaults to `default`
/// * `model_name` - Model name to deploy
/// * `version` - Version to deploy, defaults to deploying the latest "put" version
pub async fn deploy_model(
    client: &Client,
    lattice_prefix: Option<String>,
    model_name: &str,
    version: Option<String>,
) -> Result<DeployModelResponse> {
    let res = model_request(
        client,
        ModelOperation::Deploy,
        lattice_prefix,
        Some(model_name),
        serde_json::to_vec(&DeployModelRequest { version })?,
    )
    .await?;

    serde_json::from_slice(&res.payload).map_err(|e| anyhow::anyhow!(e))
}

/// Put a model definition, instructing wadm to store the application manifest for later deploys
///
/// # Arguments
/// * `client` - The [Client](async_nats::Client) to use in order to send the request message
/// * `lattice_prefix` - Optional lattice prefix that the application manifest will be stored on, defaults to `default`
/// * `model` - The full YAML or JSON string containing the OAM wadm manifest
pub async fn put_model(
    client: &Client,
    lattice_prefix: Option<String>,
    model: &str,
) -> Result<PutModelResponse> {
    let res = model_request(
        client,
        ModelOperation::Put,
        lattice_prefix,
        None,
        model.as_bytes().to_vec(),
    )
    .await?;

    serde_json::from_slice(&res.payload).map_err(|e| anyhow::anyhow!(e))
}

/// Query wadm for the history of a given model name
///
/// # Arguments
/// * `client` - The [Client](async_nats::Client) to use in order to send the request message
/// * `lattice_prefix` - Optional lattice prefix that the application manifest is stored on, defaults to `default`
/// * `model_name` - Name of the model to retrieve history for
pub async fn get_model_history(
    client: &Client,
    lattice_prefix: Option<String>,
    model_name: &str,
) -> Result<VersionResponse> {
    let res = model_request(
        client,
        ModelOperation::History,
        lattice_prefix,
        Some(model_name),
        vec![],
    )
    .await?;

    serde_json::from_slice(&res.payload).map_err(|e| anyhow::anyhow!(e))
}

/// Query wadm for details on a given model
///
/// # Arguments
/// * `client` - The [Client](async_nats::Client) to use in order to send the request message
/// * `lattice_prefix` - Optional lattice prefix that the application manifest is stored on, defaults to `default`
/// * `model_name` - Name of the model to retrieve history for
/// * `version` - Version to retrieve, defaults to retrieving the latest "put" version
pub async fn get_model_details(
    client: &Client,
    lattice_prefix: Option<String>,
    model_name: &str,
    version: Option<String>,
) -> Result<GetModelResponse> {
    let res = model_request(
        client,
        ModelOperation::Get,
        lattice_prefix,
        Some(model_name),
        serde_json::to_vec(&GetModelRequest { version })?,
    )
    .await?;

    serde_json::from_slice(&res.payload).map_err(|e| anyhow::anyhow!(e))
}

/// Delete a model version from wadm
///
/// # Arguments
/// * `client` - The [Client](async_nats::Client) to use in order to send the request message
/// * `lattice_prefix` - Optional lattice prefix that the application manifest is stored on, defaults to `default`
/// * `model_name` - Name of the model
/// * `version` - Version to retrieve, defaults to deleting the latest "put" version (or all if `delete_all` is specified)
/// * `delete_all` - Whether or not to delete all versions for a given model name
pub async fn delete_model_version(
    client: &Client,
    lattice_prefix: Option<String>,
    model_name: &str,
    version: Option<String>,
    delete_all: bool,
) -> Result<DeleteModelResponse> {
    let res = model_request(
        client,
        ModelOperation::Delete,
        lattice_prefix,
        Some(model_name),
        serde_json::to_vec(&DeleteModelRequest {
            version: version.unwrap_or_default(),
            delete_all,
        })?,
    )
    .await?;

    serde_json::from_slice(&res.payload).map_err(|e| anyhow::anyhow!(e))
}

/// Query wadm for all application manifests
///
/// # Arguments
/// * `client` - The [Client](async_nats::Client) to use in order to send the request message
/// * `lattice_prefix` - Optional lattice prefix that the application manifests are stored on, defaults to `default`
pub async fn get_models(
    client: &Client,
    lattice_prefix: Option<String>,
) -> Result<Vec<ModelSummary>> {
    let res = model_request(client, ModelOperation::List, lattice_prefix, None, vec![]).await?;

    serde_json::from_slice(&res.payload).map_err(|e| anyhow::anyhow!(e))
}

/// Helper function to make a NATS request given connection options, an operation, optional name, and bytes
/// Designed for internal use
async fn model_request(
    client: &Client,
    operation: ModelOperation,
    lattice_prefix: Option<String>,
    object_name: Option<&str>,
    bytes: Vec<u8>,
) -> Result<Message> {
    // Topic is of the form of wadm.api.<lattice>.<category>.<operation>.<OPTIONAL: object_name>
    // We let callers of this function dictate the topic after the prefix + lattice
    let topic = format!(
        "{WADM_API_PREFIX}.{}.model.{}{}",
        lattice_prefix.unwrap_or_else(|| DEFAULT_LATTICE_PREFIX.to_string()),
        operation.to_string(),
        object_name
            .map(|name| format!(".{name}"))
            .unwrap_or_default()
    );

    match tokio::time::timeout(
        Duration::from_millis(2_000),
        client.request(topic, bytes.into()),
    )
    .await
    {
        Ok(Ok(res)) => Ok(res),
        Ok(Err(e)) => bail!("Error making model request: {}", e),
        Err(e) => bail!("model_request timed out:  {}", e),
    }
}

//  NOTE(ahmedtadde): This should probably be refactored at some point to account for cases where the source's input is unusually (or erroneously) large.
pub async fn app_manifest_loader(source: &Option<String>) -> Result<AppManifest> {
    match source {
        Some(s) if PathBuf::from(s).exists() => Ok(AppManifest::SerializedModel(
            tokio::fs::read_to_string(s).await?,
        )),
        Some(s) if Url::parse(s).is_ok() && s.starts_with("http") => Ok(
            AppManifest::SerializedModel(reqwest::get(s).await?.text().await?),
        ),
        Some(s) if s == "-" => {
            let mut buffer = String::new();
            tokio::io::stdin().read_to_string(&mut buffer).await?;
            Ok(AppManifest::SerializedModel(buffer))
        }
        // NOTE(ahmedtadde): If the source is a string that isn't matched by any of the previous branches, we assume it's a model name
        // Though, applying some validation here would be nice. I looked around for existing model name validation and didn't find any.
        Some(s) => Ok(AppManifest::ModelName(s.to_owned())),
        None => {
            let mut buffer = String::new();
            tokio::io::stdin().read_to_string(&mut buffer).await?;
            Ok(AppManifest::SerializedModel(buffer))
        }
    }
}
