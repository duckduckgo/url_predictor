package com.duckduckgo.urlpredictor

import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import kotlin.concurrent.thread

class UrlPredictorTests {

    private fun classify(input: String): Decision {
        val latch = CountDownLatch(1)
        thread {
            UrlPredictor.allowInitOnMainThreadInTests = true
            UrlPredictor.init() // idempotent, it's fine to call multiple times
            latch.countDown()
        }
        latch.await()

        return UrlPredictor.get().classify(input)
    }

    @Test
    fun `return false when not initialized`() {
        UrlPredictor.destroyForTests()
        assertFalse(UrlPredictor.isInitialized())
    }

    @Test
    fun `return true when is initialized`() {
        UrlPredictor.destroyForTests()
        classify("test")
        assertTrue(UrlPredictor.isInitialized())
    }
    // ------------------------------------------------------------------------
    // Basic behavior
    // ------------------------------------------------------------------------

    @Test
    fun `single word becomes search`() {
        val d = classify("test")
        assertTrue(d is Decision.Search)
        assertEquals("test", (d as Decision.Search).query)
    }

    @Test
    fun `simple http url navigates`() {
        val d = classify("http://example.com")
        assertTrue(d is Decision.Navigate)
        assertEquals("http://example.com/", (d as Decision.Navigate).url)
    }

    // ------------------------------------------------------------------------
    // Telephone numbers (mirrors new Rust test)
    // ------------------------------------------------------------------------

    @Test
    fun `telephone number plain digits is search`() {
        val d = classify("912345678")
        assertTrue(d is Decision.Search)
        assertEquals("912345678", (d as Decision.Search).query)
    }

    @Test
    fun `telephone number with intl forma  tting is search`() {
        val input = "+351 912 345 678"
        val d = classify(input)
        assertTrue(d is Decision.Search)
        assertEquals(input, (d as Decision.Search).query)
    }

    // ------------------------------------------------------------------------
    // Ports, userinfo, edge-cases (mirrors Rust tests)
    // ------------------------------------------------------------------------

    @Test
    fun `host with port navigates`() {
        val d = classify("example.com:8080")
        assertTrue(d is Decision.Navigate)
    }

    @Test
    fun `userinfo forces navigate`() {
        val d = classify("user:pass@example.com")
        assertTrue(d is Decision.Navigate)
    }

    // ------------------------------------------------------------------------
    // Schemes allowed vs disallowed
    // ------------------------------------------------------------------------

    @Test
    fun `allowed scheme navigates`() {
        val d = classify("ftp://example.com")
        assertTrue(d is Decision.Navigate)
    }

    // ------------------------------------------------------------------------
    // Single label intranet behaviors
    // ------------------------------------------------------------------------

    @Test
    fun `single label host becomes search by default`() {
        val d = classify("intranet")
        assertTrue(d is Decision.Search)
    }

    @Test
    fun `multi label intranet allowed when enabled`() {
        val d = classify("router.local")
        assertTrue(d is Decision.Navigate)
    }

    // ------------------------------------------------------------------------
    // Unicode, invalid labels (mirrors Rust)
    // ------------------------------------------------------------------------

    @Test
    fun `unicode IDNA host navigates`() {
        val d = classify("https://bücher.example")
        assertTrue(d is Decision.Navigate)
    }

    @Test
    fun `invalid schemed-hostname navigates`() {
        val d = classify("http://-badlabel.com")
        assertTrue(d is Decision.Navigate)
    }

    @Test
    fun `invalid bare-hostname becomes search`() {
        val d = classify("-badlabel.com")
        assertTrue(d is Decision.Search)
    }

    @Test
    fun `chrome scheme becomes search`() {
        val d = classify("chrome://flags")
        assertTrue(d is Decision.Search)
    }

    @Test
    fun `edge scheme becomes search`() {
        val d = classify("edge://flags")
        assertTrue(d is Decision.Search)
    }

    @Test
    fun `duck scheme navigates`() {
        val d = classify("duck://flags")
        assertTrue(d is Decision.Navigate)
    }

    // ------------------------------------------------------------------------
    // mailto URLs
    // ------------------------------------------------------------------------

    @Test
    fun `mailto google becomes search`() {
        val d = classify("mailto:test@google.com")
        assertTrue(d is Decision.Navigate)
    }

    @Test
    fun `mailto yahoo becomes search`() {

        val d = classify("mailto:test@yahoo.com")
        assertTrue(d is Decision.Navigate)
    }

    // ------------------------------------------------------------------------
    // known bare domains
    // ------------------------------------------------------------------------
    @Test
    fun `known bare domains navigate`() {
        assertTrue(classify("blogspot.com") is Decision.Navigate)
        assertTrue(classify("github.io") is Decision.Navigate)
        assertTrue(classify("gov.cz") is Decision.Navigate)
        assertTrue(classify("gov.pl") is Decision.Navigate)
    }
}
