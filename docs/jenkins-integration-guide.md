# Jenkins CI/CD Integration Guide

**Date**: 2026-05-21  
**Status**: Design Document

## Overview

This guide describes how to integrate Jenkins with ADAM to automatically:
1. Create `pipeline_run` assets when CI pipelines execute
2. Establish dependency relationships between `pipeline_run` and `code_commit`
3. Enable full traceability: Pipeline Run → Code Commit → Work Item → Requirement

## Integration Approaches

### Option 1: Jenkins Shared Library (Recommended)

Create a reusable Jenkins Shared Library that pipeline scripts can import.

**Pros**: Easy to use, version controlled, minimal configuration  
**Cons**: Requires Shared Library setup

### Option 2: Direct REST API Calls (curl)

Use `curl` commands directly in `Jenkinsfile`.

**Pros**: No additional setup, works immediately  
**Cons**: Verbose, error handling is manual

### Option 3: Custom Jenkins Plugin

Develop a Jenkins plugin for ADAM integration.

**Pros**: Native UI integration, best user experience  
**Cons**: Requires Java development, longer implementation time

---

## Option 1: Jenkins Shared Library (Recommended)

### Step 1: Create Shared Library

**Directory structure**:
```
vars/
  adamPipelineRun.groovy
  adamCreateAsset.groovy
src/
  org/adam/
    AdamClient.groovy
resources/
  adam/
    reportTemplate.html
```

### Step 2: Implementation

**vars/adamPipelineRun.groovy**:
```groovy
#!/usr/bin/env groovy

/**
 * Create ADAM pipeline_run asset and link to code commit
 *
 * @param config Map with:
 *   - adamUrl: ADAM server URL (default: http://localhost:3000)
 *   - apiToken: ADAM API token
 *   - orgId: Organization ID
 *   - projectId: Project ID (optional for org-level)
 *   - pipelineRunTypeId: Asset type ID for pipeline_run
 *   - codeCommitTypeId: Asset type ID for code_commit
 *   - commitAssetId: Existing code_commit asset ID (optional)
 *   - createCommitIfMissing: Boolean - create code_commit if not found (default: true)
 *   - metadata: Additional metadata for pipeline_run
 *
 * @return Map with pipelineRunId, commitAssetId, success status
 */
def call(Map config = [:]) {
    def adamUrl = config.adamUrl ?: env.ADAM_URL ?: 'http://localhost:3000'
    def apiToken = config.apiToken ?: env.ADAM_API_TOKEN
    def orgId = config.orgId ?: env.ADAM_ORG_ID
    def projectId = config.projectId ?: env.ADAM_PROJECT_ID
    
    if (!apiToken || !orgId) {
        error "ADAM_API_TOKEN and ADAM_ORG_ID are required"
    }
    
    def commitHash = env.GIT_COMMIT ?: sh(script: 'git rev-parse HEAD', returnStdout: true).trim()
    def commitMsg = env.GIT_COMMIT_MESSAGE ?: sh(script: 'git log -1 --pretty=%B', returnStdout: true).trim()
    def commitAuthor = env.GIT_AUTHOR_NAME ?: sh(script: 'git log -1 --pretty=%an', returnStdout: true).trim()
    def branchName = env.GIT_BRANCH ?: env.BRANCH_NAME ?: sh(script: 'git rev-parse --abbrev-ref HEAD', returnStdout: true).trim()
    def buildNumber = env.BUILD_NUMBER ?: '0'
    def buildUrl = env.BUILD_URL ?: ''
    
    echo "Creating ADAM pipeline_run for commit ${commitHash}"
    
    def result = [
        success: false,
        pipelineRunId: null,
        commitAssetId: null,
        errors: []
    ]
    
    try {
        // Step 1: Find or create code_commit asset
        def commitAssetId = config.commitAssetId
        
        if (!commitAssetId && config.createCommitIfMissing != false) {
            commitAssetId = findOrCreateCodeCommit(adamUrl, apiToken, orgId, projectId, 
                config.codeCommitTypeId, commitHash, commitMsg, commitAuthor, branchName)
        }
        
        if (!commitAssetId) {
            error "Failed to find or create code_commit asset"
        }
        
        result.commitAssetId = commitAssetId
        
        // Step 2: Create pipeline_run asset
        def pipelineRunId = createPipelineRun(adamUrl, apiToken, orgId, projectId,
            config.pipelineRunTypeId, buildNumber, buildUrl, commitHash, currentBuild.result ?: 'RUNNING',
            config.metadata ?: [:])
        
        result.pipelineRunId = pipelineRunId
        
        // Step 3: Create dependency relationship
        if (commitAssetId && pipelineRunId) {
            createDependency(adamUrl, apiToken, pipelineRunId, commitAssetId)
        }
        
        result.success = true
        echo "ADAM integration successful: pipeline_run=${pipelineRunId}, commit=${commitAssetId}"
        
    } catch (Exception e) {
        result.errors << e.message
        echo "ADAM integration failed: ${e.message}"
        // Don't fail the build - ADAM is informational
    }
    
    return result
}

/**
 * Find existing code_commit by external_ref or create new one
 */
def findOrCreateCodeCommit(adamUrl, apiToken, orgId, projectId, typeId, commitHash, commitMsg, author, branch) {
    // First, try to find existing commit by external_ref
    def findUrl = "${adamUrl}/api/v1/assets?external_ref=${commitHash}&source=git"
    def findResponse = httpRequest(
        url: findUrl,
        httpMode: 'GET',
        customHeaders: [[name: 'Authorization', value: "Bearer ${apiToken}"]],
        validResponseCodes: '200'
    )
    
    def findJson = readJSON(text: findResponse.content)
    if (findJson.assets?.size() > 0) {
        echo "Found existing code_commit: ${findJson.assets[0].id}"
        return findJson.assets[0].id
    }
    
    // Create new code_commit asset
    def createUrl = "${adamUrl}/api/v1/assets"
    def payload = [
        name: "Commit ${commitHash.substring(0, 7)}: ${commitMsg.take(50)}",
        asset_type_id: typeId ?: '44444444-4444-4444-4444-444444444444', // default code_commit type
        project_id: projectId,
        level: 'project',
        external_ref: commitHash,
        source: 'git',
        metadata: [
            hash: commitHash,
            message: commitMsg,
            author: author,
            branch: branch,
            timestamp: new Date().toString()
        ],
        idempotency_key: "git:${commitHash}" // Prevent duplicate creation
    ]
    
    def createResponse = httpRequest(
        url: createUrl,
        httpMode: 'POST',
        customHeaders: [[name: 'Authorization', value: "Bearer ${apiToken}"]],
        requestBody: groovy.json.JsonOutput.toJson(payload),
        validResponseCodes: '201'
    )
    
    def createJson = readJSON(text: createResponse.content)
    echo "Created code_commit: ${createJson.id}"
    return createJson.id
}

/**
 * Create pipeline_run asset
 */
def createPipelineRun(adamUrl, apiToken, orgId, projectId, typeId, buildNumber, buildUrl, commitHash, status, metadata) {
    def createUrl = "${adamUrl}/api/v1/assets"
    
    def duration = currentBuild.duration ?: 0
    def trigger = currentBuild.buildCauses?.collect { it.shortDescription }?.join(', ') ?: 'manual'
    
    def payload = [
        name: "Build #${buildNumber}",
        asset_type_id: typeId ?: '55555555-5555-5555-5555-555555555555', // default pipeline_run type
        project_id: projectId,
        level: 'project',
        external_ref: "${env.JOB_NAME}-${buildNumber}",
        source: 'ci',
        metadata: [
            build_number: buildNumber,
            build_url: buildUrl,
            status: status,
            trigger: trigger,
            duration_ms: duration,
            commit_hash: commitHash,
            jenkins_job: env.JOB_NAME,
            jenkins_node: env.NODE_NAME,
            additional: metadata
        ],
        idempotency_key: "jenkins:${env.JOB_NAME}:${buildNumber}"
    ]
    
    def response = httpRequest(
        url: createUrl,
        httpMode: 'POST',
        customHeaders: [[name: 'Authorization', value: "Bearer ${apiToken}"]],
        requestBody: groovy.json.JsonOutput.toJson(payload),
        validResponseCodes: '201'
    )
    
    def json = readJSON(text: response.content)
    echo "Created pipeline_run: ${json.id}"
    return json.id
}

/**
 * Create dependency: pipeline_run depends on code_commit
 */
def createDependency(adamUrl, apiToken, downstreamId, upstreamId) {
    def depUrl = "${adamUrl}/api/v1/assets/${downstreamId}/dependencies"
    
    def payload = [
        upstream_asset_id: upstreamId,
        version: '*', // Final assets use external_ref, not semver
        constraint_type: 'exact'
    ]
    
    def response = httpRequest(
        url: depUrl,
        httpMode: 'POST',
        customHeaders: [[name: 'Authorization', value: "Bearer ${apiToken}"]],
        requestBody: groovy.json.JsonOutput.toJson(payload),
        validResponseCodes: '201'
    )
    
    echo "Created dependency: ${downstreamId} -> ${upstreamId}"
}
```

### Step 3: Use in Jenkinsfile

```groovy
// Jenkinsfile
@Library('adam-shared-library') _

pipeline {
    agent any
    
    environment {
        ADAM_URL = 'http://adam-server:3000'
        ADAM_API_TOKEN = credentials('adam-api-token')
        ADAM_ORG_ID = 'my-org-uuid'
        ADAM_PROJECT_ID = 'my-project-uuid'
    }
    
    stages {
        stage('Build') {
            steps {
                // Your build steps
                sh 'make build'
            }
        }
        
        stage('Test') {
            steps {
                sh 'make test'
            }
        }
    }
    
    post {
        always {
            // Create ADAM pipeline_run and link to commit
            script {
                def adamResult = adamPipelineRun(
                    pipelineRunTypeId: '55555555-5555-5555-5555-555555555555',
                    codeCommitTypeId: '44444444-4444-4444-4444-444444444444',
                    createCommitIfMissing: true,
                    metadata: [
                        test_results: 'tests/results.xml',
                        coverage: '80%'
                    ]
                )
                
                if (adamResult.success) {
                    echo "ADAM Pipeline Run: ${adamResult.pipelineRunId}"
                    echo "ADAM Commit Asset: ${adamResult.commitAssetId}"
                    
                    // Store IDs for later stages or notifications
                    env.ADAM_PIPELINE_RUN_ID = adamResult.pipelineRunId
                    env.ADAM_COMMIT_ASSET_ID = adamResult.commitAssetId
                }
            }
        }
    }
}
```

---

## Option 2: Direct curl Commands

For quick integration without Shared Library setup:

```groovy
pipeline {
    agent any
    
    environment {
        ADAM_URL = 'http://adam-server:3000'
        ADAM_TOKEN = credentials('adam-token')
    }
    
    stages {
        stage('Build') {
            steps {
                sh 'make build'
            }
        }
    }
    
    post {
        always {
            script {
                def commitHash = sh(script: 'git rev-parse HEAD', returnStdout: true).trim()
                def commitMsg = sh(script: 'git log -1 --pretty=%s', returnStdout: true).trim()
                
                // Step 1: Create code_commit asset
                def commitResponse = sh(
                    script: """
                        curl -s -X POST ${ADAM_URL}/api/v1/assets \\
                        -H "Authorization: Bearer ${ADAM_TOKEN}" \\
                        -H "Content-Type: application/json" \\
                        -d '{
                            "name": "Commit ${commitHash.take(7)}: ${commitMsg.take(30)}",
                            "asset_type_id": "44444444-4444-4444-4444-444444444444",
                            "project_id": "${env.ADAM_PROJECT_ID}",
                            "level": "project",
                            "external_ref": "${commitHash}",
                            "source": "git",
                            "metadata": {
                                "hash": "${commitHash}",
                                "message": "${commitMsg}"
                            },
                            "idempotency_key": "git:${commitHash}"
                        }'
                    """,
                    returnStdout: true
                ).trim()
                
                // Parse response to get asset ID
                def commitJson = readJSON(text: commitResponse)
                def commitAssetId = commitJson.id
                
                // Step 2: Create pipeline_run asset
                def pipelineResponse = sh(
                    script: """
                        curl -s -X POST ${ADAM_URL}/api/v1/assets \\
                        -H "Authorization: Bearer ${ADAM_TOKEN}" \\
                        -H "Content-Type: application/json" \\
                        -d '{
                            "name": "Build #${env.BUILD_NUMBER}",
                            "asset_type_id": "55555555-5555-5555-5555-555555555555",
                            "project_id": "${env.ADAM_PROJECT_ID}",
                            "level": "project",
                            "external_ref": "${env.JOB_NAME}-${env.BUILD_NUMBER}",
                            "source": "ci",
                            "metadata": {
                                "status": "${currentBuild.result ?: 'SUCCESS'}",
                                "build_number": "${env.BUILD_NUMBER}"
                            }
                        }'
                    """,
                    returnStdout: true
                ).trim()
                
                def pipelineJson = readJSON(text: pipelineResponse)
                def pipelineAssetId = pipelineJson.id
                
                // Step 3: Create dependency
                sh """
                    curl -s -X POST ${ADAM_URL}/api/v1/assets/${pipelineAssetId}/dependencies \\
                    -H "Authorization: Bearer ${ADAM_TOKEN}" \\
                    -H "Content-Type: application/json" \\
                    -d '{
                        "upstream_asset_id": "${commitAssetId}",
                        "version": "*"
                    }'
                """
            }
        }
    }
}
```

---

## Configuration

### Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `ADAM_URL` | ADAM server URL | Yes |
| `ADAM_API_TOKEN` | API authentication token | Yes |
| `ADAM_ORG_ID` | Organization ID | Yes |
| `ADAM_PROJECT_ID` | Project ID | Optional (for project-level assets) |

### Jenkins Credentials

Store API token securely:

```groovy
// Add credential in Jenkins UI:
// Kind: Secret text
// Secret: {your-api-token}
// ID: adam-api-token

// Reference in pipeline:
environment {
    ADAM_TOKEN = credentials('adam-api-token')
}
```

---

## Troubleshooting

### Common Issues

**1. Asset type not found**
```
Error: Asset type 'pipeline_run' not found
```
- Run migration `012_default_dependency_rules.sql`
- Verify asset types exist: `GET /api/v1/asset-types`

**2. Duplicate idempotency key**
```
Error: Duplicate idempotency key
```
- Use unique idempotency keys per build
- Include build number or timestamp

**3. Authentication failed**
```
Error: 401 Unauthorized
```
- Check API token format: `{org_id}:{user_id}:{role}:{projects}`
- Verify token is base64 encoded if required

---

## Future Enhancements

1. **Jenkins Plugin**: Native plugin with UI configuration
2. **Webhook Support**: ADAM calls Jenkins webhook when dependencies change
3. **Build Status Sync**: Update pipeline_run status as build progresses
4. **Test Report Integration**: Parse JUnit XML and link test cases to commits
5. **Coverage Integration**: Link code coverage reports to commits

---

## Related Documentation

- [REST API Reference](rest-api.md)
- [Default Dependency Rules Migration](../migrations/012_default_dependency_rules.sql)
- [Final State Design](2026-05-21-final-state-design.md)
