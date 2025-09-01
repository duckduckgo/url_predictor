package com.duckduckgo.urlpredictor

object UrlPredictor {
    init { System.loadLibrary("url_predictor") }

    // Low-level JNI (returns JSON from Rust)
    @JvmStatic private external fun ddgClassifyJni(input: String, policyJson: String): String

    // High-level, type-safe API
    @JvmStatic
    fun classify(input: String): Decision {
        return classifyInternal(input)
    }

    @JvmStatic
    private fun classifyInternal(input: String, policy: DecisionJson.Policy = DecisionJson.Policy()): Decision {
        val policyJson = DecisionJson.encodePolicy(policy)
        val decisionJson = ddgClassifyJni(input, policyJson)
        return DecisionJson.decodeDecision(decisionJson)
    }
}
