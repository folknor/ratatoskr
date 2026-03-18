use super::{
    EWS_URL, EwsClient, EwsFolder, EwsHeaders, EwsItem, FindItemsResult,
    build_soap_envelope, check_soap_fault, is_distinguished_folder_id,
    parse_create_item_response, parse_find_folder_response, parse_find_items_response,
    parse_get_folder_response, parse_get_item_response, xml_escape,
};

impl Default for EwsClient {
    fn default() -> Self {
        Self {
            http: reqwest::Client::new(),
            ews_url: EWS_URL.to_string(),
        }
    }
}

impl EwsClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_url(url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            ews_url: url,
        }
    }

    /// Execute a raw EWS SOAP request. Wraps `body_xml` in the SOAP envelope,
    /// sends it, checks for SOAP faults, and returns the response body.
    pub async fn execute(
        &self,
        access_token: &str,
        body_xml: &str,
        headers: Option<&EwsHeaders>,
    ) -> Result<String, String> {
        let envelope = build_soap_envelope(body_xml);

        let mut req = self
            .http
            .post(&self.ews_url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .header("Authorization", format!("Bearer {access_token}"));

        if let Some(h) = headers {
            if let Some(ref anchor) = h.anchor_mailbox {
                req = req.header("X-AnchorMailbox", anchor);
            }
            if let Some(ref pf) = h.public_folder_mailbox {
                req = req.header("X-PublicFolderMailbox", pf);
            }
        }

        let resp = req
            .body(envelope)
            .send()
            .await
            .map_err(|e| format!("EWS request failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("EWS returned {status}: {body}"));
        }

        let xml = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read EWS response: {e}"))?;

        check_soap_fault(&xml)?;
        Ok(xml)
    }

    // ── Operations ──────────────────────────────────────────

    /// Browse child folders under a parent folder.
    /// Use `"publicfoldersroot"` for the top-level public folder hierarchy.
    pub async fn find_folder(
        &self,
        access_token: &str,
        parent_folder_id: &str,
        headers: Option<&EwsHeaders>,
    ) -> Result<Vec<EwsFolder>, String> {
        let distinguished = is_distinguished_folder_id(parent_folder_id);
        let escaped_id = xml_escape(parent_folder_id);
        let parent_xml = if distinguished {
            format!(r#"<t:DistinguishedFolderId Id="{escaped_id}"/>"#)
        } else {
            format!(r#"<t:FolderId Id="{escaped_id}"/>"#)
        };

        let body_xml = format!(
            r#"<m:FindFolder Traversal="Shallow">
  <m:FolderShape>
    <t:BaseShape>Default</t:BaseShape>
    <t:AdditionalProperties>
      <t:FieldURI FieldURI="folder:EffectiveRights"/>
      <t:FieldURI FieldURI="folder:FolderClass"/>
    </t:AdditionalProperties>
  </m:FolderShape>
  <m:ParentFolderIds>
    {parent_xml}
  </m:ParentFolderIds>
</m:FindFolder>"#
        );

        let xml = self.execute(access_token, &body_xml, headers).await?;
        parse_find_folder_response(&xml)
    }

    /// Get detailed info for a single folder, including `PR_REPLICA_LIST`.
    pub async fn get_folder(
        &self,
        access_token: &str,
        folder_id: &str,
        headers: Option<&EwsHeaders>,
    ) -> Result<EwsFolder, String> {
        let escaped_id = xml_escape(folder_id);
        let body_xml = format!(
            r#"<m:GetFolder>
  <m:FolderShape>
    <t:BaseShape>Default</t:BaseShape>
    <t:AdditionalProperties>
      <t:FieldURI FieldURI="folder:EffectiveRights"/>
      <t:FieldURI FieldURI="folder:FolderClass"/>
      <t:ExtendedFieldURI PropertyTag="0x6698" PropertyType="Binary"/>
    </t:AdditionalProperties>
  </m:FolderShape>
  <m:FolderIds>
    <t:FolderId Id="{escaped_id}"/>
  </m:FolderIds>
</m:GetFolder>"#
        );

        let xml = self.execute(access_token, &body_xml, headers).await?;
        parse_get_folder_response(&xml)
    }

    /// Find items (messages) in a folder with paging and optional date filter.
    pub async fn find_items(
        &self,
        access_token: &str,
        folder_id: &str,
        since: Option<&str>,
        offset: u32,
        max_entries: u32,
        headers: Option<&EwsHeaders>,
    ) -> Result<FindItemsResult, String> {
        let restriction = match since {
            Some(dt) => {
                let escaped_dt = xml_escape(dt);
                format!(
                    r#"<m:Restriction>
    <t:IsGreaterThanOrEqualTo>
      <t:FieldURI FieldURI="item:DateTimeReceived"/>
      <t:FieldURIOrConstant>
        <t:Constant Value="{escaped_dt}"/>
      </t:FieldURIOrConstant>
    </t:IsGreaterThanOrEqualTo>
  </m:Restriction>"#
                )
            }
            None => String::new(),
        };

        let escaped_folder_id = xml_escape(folder_id);
        let body_xml = format!(
            r#"<m:FindItem Traversal="Shallow">
  <m:ItemShape>
    <t:BaseShape>IdOnly</t:BaseShape>
    <t:AdditionalProperties>
      <t:FieldURI FieldURI="item:Subject"/>
      <t:FieldURI FieldURI="item:DateTimeReceived"/>
      <t:FieldURI FieldURI="message:From"/>
      <t:FieldURI FieldURI="item:Preview"/>
      <t:FieldURI FieldURI="message:IsRead"/>
      <t:FieldURI FieldURI="item:ItemClass"/>
    </t:AdditionalProperties>
  </m:ItemShape>
  <m:IndexedPageItemView MaxEntriesReturned="{max_entries}" Offset="{offset}" BasePoint="Beginning"/>
  {restriction}
  <m:SortOrder>
    <t:FieldOrder Order="Descending">
      <t:FieldURI FieldURI="item:DateTimeReceived"/>
    </t:FieldOrder>
  </m:SortOrder>
  <m:ParentFolderIds>
    <t:FolderId Id="{escaped_folder_id}"/>
  </m:ParentFolderIds>
</m:FindItem>"#
        );

        let xml = self.execute(access_token, &body_xml, headers).await?;
        parse_find_items_response(&xml)
    }

    /// Get full details of a single item (message).
    pub async fn get_item(
        &self,
        access_token: &str,
        item_id: &str,
        headers: Option<&EwsHeaders>,
    ) -> Result<EwsItem, String> {
        let escaped_id = xml_escape(item_id);
        let body_xml = format!(
            r#"<m:GetItem>
  <m:ItemShape>
    <t:BaseShape>Default</t:BaseShape>
    <t:AdditionalProperties>
      <t:FieldURI FieldURI="item:Body"/>
      <t:FieldURI FieldURI="item:Attachments"/>
      <t:FieldURI FieldURI="message:ToRecipients"/>
      <t:FieldURI FieldURI="message:CcRecipients"/>
    </t:AdditionalProperties>
    <t:BodyType>HTML</t:BodyType>
  </m:ItemShape>
  <m:ItemIds>
    <t:ItemId Id="{escaped_id}"/>
  </m:ItemIds>
</m:GetItem>"#
        );

        let xml = self.execute(access_token, &body_xml, headers).await?;
        parse_get_item_response(&xml)
    }

    /// Create a new message item in a public folder.
    pub async fn create_item(
        &self,
        access_token: &str,
        folder_id: &str,
        subject: &str,
        body_html: &str,
        to_recipients: &[(&str, &str)],
        headers: Option<&EwsHeaders>,
    ) -> Result<String, String> {
        let mut recipients_xml = String::new();
        for (email, name) in to_recipients {
            let escaped_email = xml_escape(email);
            let escaped_name = xml_escape(name);
            recipients_xml.push_str(&format!(
                r#"<t:Mailbox>
          <t:EmailAddress>{escaped_email}</t:EmailAddress>
          <t:Name>{escaped_name}</t:Name>
        </t:Mailbox>"#
            ));
        }

        let escaped_subject = xml_escape(subject);
        let escaped_body = xml_escape(body_html);
        let escaped_folder_id = xml_escape(folder_id);

        let body_xml = format!(
            r#"<m:CreateItem MessageDisposition="SaveOnly">
  <m:SavedItemFolderId>
    <t:FolderId Id="{escaped_folder_id}"/>
  </m:SavedItemFolderId>
  <m:Items>
    <t:Message>
      <t:Subject>{escaped_subject}</t:Subject>
      <t:Body BodyType="HTML">{escaped_body}</t:Body>
      <t:ToRecipients>
        {recipients_xml}
      </t:ToRecipients>
    </t:Message>
  </m:Items>
</m:CreateItem>"#
        );

        let xml = self.execute(access_token, &body_xml, headers).await?;
        parse_create_item_response(&xml)
    }
}
