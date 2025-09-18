package com.duckduckgo.urlpredictor

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject

sealed interface Decision {
    @Serializable data class Navigate(val url: String) : Decision
    @Serializable data class Search(val query: String) : Decision
}

object DecisionJson {
    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = true
        prettyPrint = false
    }

    fun decodeDecision(jsonStr: String): Decision {
        val root = json.parseToJsonElement(jsonStr).jsonObject
        // Expect exactly one entry: "Navigate" | "Search"
        val (tag, payloadEl) = root.entries.first()
        return when (tag) {
            "Navigate" -> json.decodeFromJsonElement(Decision.Navigate.serializer(), payloadEl)
            "Search"   -> json.decodeFromJsonElement(Decision.Search.serializer(), payloadEl)
            else -> error("Unknown decision: $tag")
        }
    }

    fun encodePolicy(policy: Policy): String = json.encodeToString(Policy.serializer(), policy)

    @Serializable
    data class Policy(
        val allow_intranet_multi_label: Boolean = false,
        val allow_intranet_single_label: Boolean = false,
        val allow_private_suffix: Boolean = true,
        val allowed_schemes: Set<String> = setOf(
            "http", "https",
            "ftp",
            "file",
            "about",
            "view-source",
            "mailto", "tel", "sms" // optional, if you want these to navigate
        ),
    )
}
