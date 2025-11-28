package com.duckduckgo.urlpredictor

import android.os.Looper

class UrlPredictor {

    companion object {
        private val defaultPolicy = DecisionJson.Policy()
        private val defaultPolicyJson: String by lazy {
            DecisionJson.encodePolicy(defaultPolicy)
        }

        @Volatile private var instance: UrlPredictor? = null

        @Volatile
        private var initialized = false

        internal var allowInitOnMainThreadInTests = false

        internal fun destroyForTests() {
            instance = null
            initialized = false
        }


        /**
         * Safe, idempotent initialization.
         * It MUST not be called on the main thread
         *
         * - Can be called multiple times.
         * - Thread-safe.
         * - Only the **first** call invokes System.loadLibrary().
         * - Subsequent calls are no-ops.
         */
        fun init() {
            fun checkMainThread() {
                if (!allowInitOnMainThreadInTests && (Looper.getMainLooper() == Looper.myLooper())) {
                    "UrlPredictor.init() must not be called on main thread"
                }
            }

            checkMainThread()

            if (!initialized) {
                synchronized(this) {
                    if (!initialized) {
                        System.loadLibrary("url_predictor")
                        instance = UrlPredictor()
                        initialized = true
                    }
                }
            }
        }

        /**
         * Returns whether the UrlPredictor native library has been fully initialized.
         *
         * Initialization is performed by calling {@link #init()}, which loads the
         * underlying native library via {@code System.loadLibrary("url_predictor")}
         * and constructs the singleton UrlPredictor instance.
         *
         * This method is safe to call from any thread and does not perform any
         * synchronization; it simply returns the current value of an internal
         * volatile flag.
         *
         * @return {@code true} if {@link #init()} has completed successfully and
         *         the native library is ready for use; {@code false} if initialization
         *         has not yet occurred or is still in progress.
         *
         * @see init
         * @see get
         */
        fun isInitialized(): Boolean = initialized

        fun get(): UrlPredictor = instance
            ?: error("UrlPredictor.init() not called")
    }

    // Low-level JNI (returns JSON from Rust)
    private external fun ddgClassifyJni(input: String, policyJson: String): String

    // High-level, type-safe API
    fun classify(input: String): Decision {
        return classifyInternal(input)
    }

    private fun classifyInternal(input: String, policy: DecisionJson.Policy = defaultPolicy): Decision {
        // for now we don't want to expose the default policy in the public API, so optimising a bit
        val policyJson = if (policy === defaultPolicy) {
            defaultPolicyJson
        } else {
            DecisionJson.encodePolicy(policy)
        }
        val decisionJson = ddgClassifyJni(input, policyJson)
        return DecisionJson.decodeDecision(decisionJson)
    }
}
