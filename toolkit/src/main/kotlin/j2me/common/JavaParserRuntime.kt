package j2me.common

import com.github.javaparser.JavaParser
import com.github.javaparser.ParseProblemException
import com.github.javaparser.ParseResult
import com.github.javaparser.ParserConfiguration
import com.github.javaparser.ast.CompilationUnit
import com.github.javaparser.ast.Node
import com.github.javaparser.ast.type.Type

private val javaParserConfig = ParserConfiguration().apply {
    // JavaParser validators/post-processors use reflective metamodel field access.
    // We only need syntax trees, so keep processors disabled.
    processors.clear()
}

private val threadLocalJavaParser = ThreadLocal.withInitial {
    JavaParser(javaParserConfig)
}

private fun parser(): JavaParser = threadLocalJavaParser.get()

private fun <N : Node> requireParsed(result: ParseResult<N>): N {
    if (!result.isSuccessful) {
        throw ParseProblemException(result.problems)
    }
    return result.result.orElseThrow { ParseProblemException(result.problems) }
}

fun parseCompilationUnit(source: String): CompilationUnit = requireParsed(parser().parse(source))

fun parseTypeNode(source: String): Type = requireParsed(parser().parseType(source))
