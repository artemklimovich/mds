package ru.pchelopoisk.app

object TrustedNodes {
    const val VERSION = "0.1.11"
    val hives = listOf("https://45.158.46.52")
    fun hive(): String = hives.first()
}
